//! The shell: layout, drawing and input of the ridge, the launcher, the
//! wallpaper, the notifications, the window switcher and the snap preview,
//! as two compositor layers ([`Shell::below`] under the windows,
//! [`Shell::above`] over them), and the settings window's contents.
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────────────────┐
//!  │ ◌                                   [● 2] [ ▮▮▮ ◖) ] 21:41 Sep 12 │ ← the ridge, 24 px, rgb(32,32,32)
//!  ├──────────────────────────────────────────────────────────────────┤
//!  │                                            ╭──────────────────╮  │
//!  │              ╭───────────────────────╮      │ ● Hello           │ ← toasts, top right
//!  │              │ ⌕  Search apps         │      │   From a client   │  │
//!  │              ├───────────────────────┤      ╰──────────────────╯  │
//!  │              │ ▢ Poly Demo            │ ← the launcher (or the    │
//!  │              ╰───────────────────────╯    switcher, the history)  │
//!  └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The ridge is a status strip (the owner's decision, ADR 0107): its
//! parts open nothing that hangs from it. The unread count opens the
//! notification history and the status pill the settings window, both in
//! the middle of the screen.
//!
//! Coordinates are logical; drawing multiplies them by the output's scale.
//! Everything that changes damages its rectangle ([`Shell::take_damage`]),
//! which the compositor hands to the scene, so only that is drawn again.

use crate::apps::App;
use crate::draw::{self, Text, fill, rounded_outline, rounded_rect};
use crate::launcher::{Key, Launcher, Outcome};
use crate::notify::{Notification, Notifications};
use crate::settings::{Change, Settings};
use crate::settings_ui::{Context, Page, SettingsWindow, SystemInfo};
use crate::switcher::{Entry, Switcher, look as sw};
use crate::theme::{launcher as lt, notice as nt, overlay as ot, ridge as rt, wallpaper as wt};
use crate::tz::TimeZone;
use alloc::string::String;
use alloc::vec::Vec;
use poly_dbus::notifications::{Notify, Urgency};
use poly_wayland::compose::{Framebuffer, Layer, Rect, WindowId};

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
    /// Not the shell's: the windows get it.
    NotMine,
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

/// The launcher's parts, logical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LauncherLayout {
    panel: Rect,
    search: Rect,
    /// The first row's rectangle; the others follow every
    /// `ROW_HEIGHT + ROW_GAP`.
    first_row: Rect,
    rows: usize,
    /// "No apps match" instead of rows.
    empty: bool,
    footer: Option<Rect>,
}

/// The ridge's parts, logical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RidgeLayout {
    clock: Rect,
    status: Rect,
    /// The unread notifications, when there are some.
    unread: Option<Rect>,
}

/// A toast's parts, logical.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ToastLayout {
    id: u32,
    card: Rect,
    close: Rect,
    title: String,
    lines: Vec<String>,
    /// (button, action key, label).
    buttons: Vec<(Rect, String, String)>,
    critical: bool,
}

/// The notification history's parts, logical.
#[derive(Clone, Debug, PartialEq, Eq)]
struct HistoryLayout {
    panel: Rect,
    clear: Option<Rect>,
    rows: Vec<Rect>,
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
    damage: Vec<Rect>,
    settings: Settings,
    zone: TimeZone,
    network: Network,
    /// The master volume and mute (`None`: no sound card).
    volume: Option<(u32, bool)>,
    notes: Notifications,
    now_ms: u64,
    history: bool,
    switcher: Switcher,
    snap: Option<Rect>,
    /// The settings window's state (its contents are the shell's).
    pub settings_window: SettingsWindow,
}

impl Shell {
    /// A shell on a `width` × `height` logical output with these apps and
    /// fonts. The launcher shows at the start (an empty desktop).
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
            damage: Vec::new(),
            settings,
            zone,
            network: Network::Unknown,
            volume: None,
            notes: Notifications::new(),
            now_ms: 0,
            history: false,
            switcher: Switcher::default(),
            snap: None,
            settings_window: SettingsWindow::default(),
        };
        let rows = s.room_for_rows();
        s.launcher.set_limit(rows);
        s.launcher.show();
        s
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

    /// The output's logical size changed (another scale or mode).
    pub fn resize(&mut self, width: i32, height: i32) {
        (self.width, self.height) = (width, height);
        let rows = self.room_for_rows();
        self.launcher.set_limit(rows);
        self.damage.push(self.screen());
    }

    /// The clock's text now.
    pub fn clock(&self) -> &str {
        &self.clock
    }

    /// The time is `secs` on the Unix clock (`offset_minutes` east of UTC):
    /// the ridge's clock shows it. True if the text changed.
    pub fn set_time(&mut self, secs: i64, offset_minutes: i32) -> bool {
        let text = crate::clock::ridge_text(&crate::clock::civil(secs, offset_minutes));
        if text == self.clock {
            return false;
        }
        self.clock = text;
        self.damage.push(self.ridge_rect());
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
        true
    }

    pub fn volume(&self) -> Option<(u32, bool)> {
        self.volume
    }

    // -----------------------------------------------------------------
    // Notifications.

    /// Run `f`, damaging what the toasts covered before and after (and the
    /// ridge's unread count, the history panel).
    fn notes_change<R>(&mut self, f: impl FnOnce(&mut Shell) -> R) -> R {
        let before = (self.toasts_extent(), self.history_extent(), self.notes.unread());
        let r = f(self);
        if self.notes.take_changed() {
            let after = (self.toasts_extent(), self.history_extent(), self.notes.unread());
            for e in [before.0, after.0, before.1, after.1].into_iter().flatten() {
                self.damage.push(e);
            }
            if before.2 != after.2 {
                self.damage.push(self.ridge_rect());
            }
        }
        r
    }

    /// The clock for timeouts, in milliseconds (any monotonic clock).
    pub fn set_now_ms(&mut self, now: u64) {
        self.now_ms = now;
        self.notes_change(|s| s.notes.tick(now));
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

    /// When a toast expires next (on the [`Shell::set_now_ms`] clock).
    pub fn next_notification_deadline(&self) -> Option<u64> {
        self.notes.next_deadline()
    }

    pub fn notifications(&self) -> &Notifications {
        &self.notes
    }

    pub fn history_open(&self) -> bool {
        self.history
    }

    /// Open or close the notification history (it opens in the middle;
    /// opening marks everything read).
    pub fn toggle_history(&mut self) {
        let before = self.history_extent();
        self.history = !self.history;
        if self.history {
            self.hide_launcher();
            self.notes.mark_read();
            self.notes.take_changed();
            self.damage.push(self.ridge_rect());
        }
        for e in [before, self.history_extent()].into_iter().flatten() {
            self.damage.push(e);
        }
    }

    /// A key while an overlay (the history) is open; true if taken.
    pub fn overlay_key(&mut self, key: Key) -> bool {
        if self.history && key == Key::Escape {
            self.toggle_history();
            return true;
        }
        false
    }

    // -----------------------------------------------------------------
    // The switcher and the snap preview.

    pub fn switcher_open(&self) -> bool {
        self.switcher.is_open()
    }

    pub fn switcher(&self) -> &Switcher {
        &self.switcher
    }

    fn switcher_extent(&self) -> Option<Rect> {
        if !self.switcher.is_open() {
            return None;
        }
        let (p, _) = self.switcher.layout(self.width, self.height);
        Some(shadowed(p))
    }

    fn switcher_change(&mut self, f: impl FnOnce(&mut Switcher)) {
        let before = self.switcher_extent();
        f(&mut self.switcher);
        for e in [before, self.switcher_extent()].into_iter().flatten() {
            self.damage.push(e);
        }
    }

    /// Open the switcher on `entries` (most recently used first).
    pub fn open_switcher(&mut self, entries: Vec<Entry>, backward: bool) {
        self.hide_launcher();
        if self.history {
            self.toggle_history();
        }
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

    /// How many rows fit below the search row on this screen.
    fn room_for_rows(&self) -> usize {
        let top = self.search_top();
        let fixed = lt::SEARCH_HEIGHT + 1 + lt::SECTION_HEIGHT + lt::LIST_PADDING + 1 + lt::FOOTER_HEIGHT + 16;
        let room = (self.height - top - fixed + lt::ROW_GAP) / (lt::ROW_HEIGHT + lt::ROW_GAP);
        (room.max(1) as usize).min(lt::MAX_ROWS)
    }

    /// The search row's top: centred, raised a little.
    fn search_top(&self) -> i32 {
        let centred = (self.height - lt::SEARCH_HEIGHT) / 2 - self.height * lt::RAISE_PERCENT / 100;
        centred.max(rt::HEIGHT + 16)
    }

    fn launcher_layout(&self) -> LauncherLayout {
        let w = lt::WIDTH.min(self.width - 2 * lt::SIDE_MARGIN).max(200);
        let x = (self.width - w) / 2;
        let top = self.search_top();
        let search = Rect::new(x, top, w, lt::SEARCH_HEIGHT);
        let n = self.launcher.results().len();
        let typed = !self.launcher.query().is_empty();
        let list_top = search.bottom() + 1 + lt::SECTION_HEIGHT;
        let first_row = Rect::new(x + lt::LIST_PADDING, list_top, w - 2 * lt::LIST_PADDING, lt::ROW_HEIGHT);
        let (height, footer) = if !typed {
            (lt::SEARCH_HEIGHT, None)
        } else {
            let rows = n.max(1) as i32;
            let list = lt::SECTION_HEIGHT + rows * lt::ROW_HEIGHT + (rows - 1) * lt::ROW_GAP + lt::LIST_PADDING;
            let body = lt::SEARCH_HEIGHT + 1 + list;
            (body + 1 + lt::FOOTER_HEIGHT, Some(Rect::new(x, top + body + 1, w, lt::FOOTER_HEIGHT)))
        };
        LauncherLayout {
            panel: Rect::new(x, top, w, height),
            search,
            first_row,
            rows: n,
            empty: typed && n == 0,
            footer,
        }
    }

    /// Everything the launcher draws on, shadow included.
    fn launcher_extent(&self) -> Rect {
        shadowed(self.launcher_layout().panel)
    }

    /// Run `f` on the launcher, damaging what it covered before and after.
    fn change(&mut self, f: impl FnOnce(&mut Shell) -> Outcome) -> Outcome {
        let was_visible = self.launcher.is_visible();
        let before = self.launcher_extent();
        let out = f(self);
        if out != Outcome::Unchanged || was_visible != self.launcher.is_visible() {
            if was_visible {
                self.damage.push(before);
            }
            if self.launcher.is_visible() {
                self.damage.push(self.launcher_extent());
            }
        }
        out
    }

    /// The hotkey: show the launcher, or hide it.
    pub fn toggle_launcher(&mut self) {
        if self.history && !self.launcher.is_visible() {
            self.toggle_history();
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
        let l = self.launcher_layout();
        if l.empty || !self.launcher.is_visible() {
            return None;
        }
        (0..l.rows).find(|&i| self.row_rect(&l, i).contains(x, y))
    }

    fn row_rect(&self, l: &LauncherLayout, i: usize) -> Rect {
        l.first_row.translate(0, i as i32 * (lt::ROW_HEIGHT + lt::ROW_GAP))
    }

    /// The pointer is at logical (`x`, `y`): hovering a launcher row or a
    /// switcher card selects it.
    pub fn pointer_motion(&mut self, x: i32, y: i32) {
        if self.switcher.is_open() {
            let (_, cards) = self.switcher.layout(self.width, self.height);
            if let Some(i) = cards.iter().position(|c| c.contains(x, y))
                && self.switcher.selected() != i
            {
                self.switcher_change(|s| {
                    s.select(i);
                });
            }
            return;
        }
        if !self.launcher.is_visible() {
            return;
        }
        let row = self.row_at(x, y);
        if self.launcher.hover() != row {
            self.change(|s| if s.launcher.set_hover(row) { Outcome::Changed } else { Outcome::Unchanged });
        }
    }

    /// A press of the left button at logical (`x`, `y`). The switcher,
    /// the launcher and the history take every press while they show (a
    /// press outside closes them, as clicking Alpine's transparent overlay
    /// does); toasts and the ridge take theirs.
    pub fn press(&mut self, x: i32, y: i32) -> Press {
        if self.switcher.is_open() {
            let (_, cards) = self.switcher.layout(self.width, self.height);
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
            if !self.launcher_layout().panel.contains(x, y) {
                self.hide_launcher();
            }
            return Press::Taken;
        }
        if self.history {
            if let Some(h) = self.history_layout() {
                if h.clear.is_some_and(|c| c.contains(x, y)) {
                    self.notes_change(|s| s.notes.clear_history());
                    self.damage.push(shadowed(h.panel));
                } else if !h.panel.contains(x, y) {
                    self.toggle_history();
                }
            }
            return Press::Taken;
        }
        // A toast: its buttons invoke actions, its cross dismisses it, the
        // rest invokes the default action (or dismisses).
        if let Some(t) = self.toast_layout().into_iter().find(|t| t.card.contains(x, y)) {
            let now = self.now_ms;
            let id = t.id;
            if let Some((_, key, _)) = t.buttons.iter().find(|b| b.0.contains(x, y)) {
                let key = key.clone();
                self.notes_change(|s| s.notes.invoke(id, &key, now));
            } else if !t.close.contains(x, y) && self.notes.has_action(id, "default") {
                self.notes_change(|s| s.notes.invoke(id, "default", now));
            } else {
                self.notes_change(|s| s.notes.dismiss(id, now));
            }
            return Press::Taken;
        }
        if self.ridge_rect().contains(x, y) {
            let r = self.ridge_layout();
            if r.unread.is_some_and(|u| u.contains(x, y)) || r.clock.contains(x, y) {
                self.toggle_history();
            } else if r.status.contains(x, y) {
                return Press::Settings(Page::Sound);
            }
            return Press::Taken;
        }
        Press::NotMine
    }

    /// The layer under the windows: the wallpaper and the ridge.
    pub fn below(&self) -> Below<'_> {
        Below(self)
    }

    /// The layer over the windows: the snap preview, the toasts, the
    /// history, the switcher and the launcher.
    pub fn above(&self) -> Above<'_> {
        Above(self)
    }

    // -----------------------------------------------------------------
    // The settings window's contents.

    /// Draw the settings window, `w` × `h` logical at scale `s`.
    pub fn draw_settings(&self, fb: &mut dyn Framebuffer, w: i32, h: i32, s: i32, info: &SystemInfo, utc: i64) {
        let now = self.local_time(utc);
        let cx = Context { settings: &self.settings, info, text: &self.text, now: &now, utc };
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
        let cx = Context { settings: &self.settings, info, text: &self.text, now: &now, utc };
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
        let cx = Context { settings: &self.settings, info, text: &self.text, now: &now, utc };
        self.settings_window.button(x, y, pressed, w, h, &cx)
    }

    // -----------------------------------------------------------------
    // Layouts of the new parts.

    fn ridge_layout(&self) -> RidgeLayout {
        let tw = self.text.width(&self.clock, rt::TEXT_SIZE) as i32;
        let clock_w = tw + 2 * rt::PILL_PADDING;
        let pill_y = (rt::HEIGHT - rt::PILL_HEIGHT) / 2;
        let clock = Rect::new(self.width - rt::RIGHT_MARGIN - clock_w, pill_y, clock_w, rt::PILL_HEIGHT);
        let icons = 2 * rt::ICON + rt::GAP;
        let status_w = icons + 2 * rt::PILL_PADDING;
        let status = Rect::new(clock.x - rt::GAP - status_w, pill_y, status_w, rt::PILL_HEIGHT);
        let unread = (self.notes.unread() > 0).then(|| {
            let label = self.unread_label();
            let w = 6 + 6 + 6 + self.text.width(&label, 11.0) as i32 + 6;
            Rect::new(status.x - rt::GAP - w, pill_y, w, rt::PILL_HEIGHT)
        });
        RidgeLayout { clock, status, unread }
    }

    fn unread_label(&self) -> String {
        let n = self.notes.unread();
        if n == 1 { "1 new".into() } else { alloc::format!("{n} new") }
    }

    /// The toasts' cards, newest at the top.
    fn toast_layout(&self) -> Vec<ToastLayout> {
        let w = nt::WIDTH.min(self.width - 2 * nt::MARGIN);
        let x = self.width - nt::MARGIN - w;
        let mut y = rt::HEIGHT + nt::MARGIN;
        let text_x = nt::PADDING_X + nt::DOT + nt::DOT_GAP;
        let text_w = (w - text_x - nt::PADDING_X - nt::CLOSE - 6) as f32;
        let mut out = Vec::new();
        for n in self.notes.toasts() {
            let (title, body) =
                if n.summary.is_empty() { (n.app_name.clone(), &n.body) } else { (n.summary.clone(), &n.body) };
            let title = self.text.fit(&title, nt::TEXT_SIZE, text_w);
            let lines =
                if body.is_empty() { Vec::new() } else { self.text.wrap(body, nt::TEXT_SIZE, text_w, nt::MAX_LINES) };
            let actions: Vec<&(String, String)> = n.actions.iter().filter(|a| a.0 != "default").take(2).collect();
            let mut h = 2 * nt::PADDING_Y + nt::LINE * (1 + lines.len() as i32);
            if !actions.is_empty() {
                h += 8 + nt::BUTTON_HEIGHT;
            }
            let card = Rect::new(x, y, w, h);
            let close = Rect::new(
                card.right() - nt::PADDING_X - nt::CLOSE,
                card.y + nt::PADDING_Y - 2,
                nt::CLOSE + 4,
                nt::LINE + 4,
            );
            let mut bx = card.x + text_x;
            let by = card.bottom() - nt::PADDING_Y - nt::BUTTON_HEIGHT;
            let mut buttons = Vec::new();
            for (key, label) in actions {
                let bw = self.text.width(label, nt::BUTTON_SIZE) as i32 + 20;
                buttons.push((Rect::new(bx, by, bw, nt::BUTTON_HEIGHT), key.clone(), label.clone()));
                bx += bw + 6;
            }
            out.push(ToastLayout {
                id: n.id,
                card,
                close,
                title,
                lines,
                buttons,
                critical: n.urgency == Urgency::Critical,
            });
            y += h + nt::GAP;
            if y > self.height {
                break;
            }
        }
        out
    }

    fn toasts_extent(&self) -> Option<Rect> {
        let l = self.toast_layout();
        let all = l.iter().fold(Rect::default(), |a, t| a.union(&t.card));
        (!all.is_empty()).then(|| shadowed(all))
    }

    /// The history panel: centred where the launcher is.
    fn history_layout(&self) -> Option<HistoryLayout> {
        if !self.history {
            return None;
        }
        let w = 480.min(self.width - 2 * lt::SIDE_MARGIN);
        let x = (self.width - w) / 2;
        let top = self.search_top();
        let n = self.notes.history_len().min(lt::MAX_ROWS);
        let rows_h = if n == 0 { lt::ROW_HEIGHT } else { n as i32 * (lt::ROW_HEIGHT + lt::ROW_GAP) - lt::ROW_GAP };
        let panel = Rect::new(x, top, w, lt::SEARCH_HEIGHT + 1 + lt::LIST_PADDING * 2 + rows_h);
        let clear = (n > 0).then(|| {
            let cw = self.text.width("Clear", 11.0) as i32 + 16;
            Rect::new(panel.right() - lt::SEARCH_PADDING - cw, top + (lt::SEARCH_HEIGHT - 22) / 2, cw, 22)
        });
        let first = top + lt::SEARCH_HEIGHT + 1 + lt::LIST_PADDING;
        let rows = (0..n)
            .map(|i| {
                Rect::new(
                    x + lt::LIST_PADDING,
                    first + i as i32 * (lt::ROW_HEIGHT + lt::ROW_GAP),
                    w - 2 * lt::LIST_PADDING,
                    lt::ROW_HEIGHT,
                )
            })
            .collect();
        Some(HistoryLayout { panel, clear, rows })
    }

    fn history_extent(&self) -> Option<Rect> {
        self.history_layout().map(|h| shadowed(h.panel))
    }

    // -----------------------------------------------------------------
    // Drawing (physical pixels: logical × s).

    fn draw_wallpaper(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        let (top, bottom, glow) = wt::colours(&self.settings.wallpaper);
        let (w, h) = (self.width * s, self.height * s);
        let ch = |c: u32, sh: u32| ((c >> sh) & 0xff) as i32;
        // A 4×4 ordered dither (thresholds 0-15 of 16) hides the steps a
        // gradient this gentle would show in 8 bits.
        const BAYER: [i32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
        for y in clip.y..clip.bottom() {
            // Channels in 1/256 levels: the vertical gradient...
            let t = (y * 256 / h.max(1)).clamp(0, 256);
            let base = [16, 8, 0].map(|sh| ch(top, sh) * (256 - t) + ch(bottom, sh) * t);
            // ...and the glow: an ellipse around a point a third down,
            // fading out half the screen's width and height away.
            let ny = (y - h / 3) * 1024 / (h / 2).max(1);
            let row = fb.row_mut(y as u32);
            for x in clip.x..clip.right() {
                let nx = (x - w / 2) * 1024 / (w / 2).max(1);
                // 0 to 1024 at the centre, squared for a soft edge.
                let g = (1024 - (nx * nx + ny * ny) / 1024).max(0);
                let a = g * g / 1024 * 90 / 256; // up to 360 of 1024
                let dither = BAYER[((y & 3) * 4 + (x & 3)) as usize] * 16;
                let c = |i: usize, sh: u32| {
                    let v = base[i] + (ch(glow, sh) * 256 - base[i]) * a / 1024;
                    (((v + dither) >> 8).clamp(0, 255) as u32) << sh
                };
                row[x as usize] = 0xff00_0000 | c(0, 16) | c(1, 8) | c(2, 0);
            }
        }
    }

    fn draw_ridge(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        let ridge = self.ridge_rect().scale(s);
        if ridge.intersect(&clip).is_empty() {
            return;
        }
        fill(fb, clip, ridge, rt::BACKGROUND, 255);
        let cy = rt::HEIGHT * s / 2;
        let l = self.ridge_layout();
        // The menu glyph on the left: a ring (Alpine's power menu face),
        // inert.
        let r = rt::ICON * s / 2 - s;
        draw::circle(fb, clip, (rt::MENU_ICON_X + rt::ICON / 2) * s, cy, r, Some(s.max(1)), WHITE, 200);
        // The clock.
        let size = rt::TEXT_SIZE * s as f32;
        let baseline = cy + (self.text.cap_height(size) as i32) / 2;
        let clock = l.clock.scale(s);
        let tx = (clock.x + rt::PILL_PADDING * s) as f32;
        self.text.draw(fb, clip, tx, baseline, ridge.right(), &self.clock, size, rt::TEXT, 255);
        // The status pill: network and volume.
        let pill = l.status.scale(s);
        rounded_rect(fb, clip, pill, pill.height / 2, WHITE, rt::PILL_ALPHA);
        let ix = pill.x + rt::PILL_PADDING * s;
        // Network: three bars rising to the right, dim when offline.
        let bars = match self.network {
            Network::Online(_) => 230,
            Network::Unknown => 140,
            Network::Offline => 70,
        };
        for i in 0..3 {
            let bh = (4 + 4 * i) * s;
            let bar = Rect::new(ix + (1 + 4 * i) * s, cy + 5 * s - bh, 3 * s, bh);
            fill(fb, clip, bar, WHITE, bars);
        }
        if self.network == Network::Offline {
            // A slash across the bars.
            for i in 0..10 * s {
                fill(fb, clip, Rect::new(ix + s + i, cy + 5 * s - i - s, s, s), WHITE, 230);
            }
        }
        // Volume: a speaker, then a wave per third of the level; a cross
        // when muted, dim without a sound card.
        let vx = ix + (rt::ICON + rt::GAP) * s;
        let (level, muted, a) = match self.volume {
            Some((v, m)) => (v, m, 230),
            None => (0, false, 90),
        };
        fill(fb, clip, Rect::new(vx + s, cy - 2 * s, 3 * s, 4 * s), WHITE, a);
        for i in 0..4 {
            fill(fb, clip, Rect::new(vx + (4 + i) * s, cy - (2 + i) * s, s, (4 + 2 * i) * s), WHITE, a);
        }
        if muted {
            for i in 0..5 * s {
                fill(fb, clip, Rect::new(vx + 9 * s + i, cy - 2 * s - s / 2 + i, s, s), WHITE, 230);
                fill(fb, clip, Rect::new(vx + 9 * s + i, cy + 2 * s + s / 2 - i, s, s), WHITE, 230);
            }
        } else {
            let waves = match level {
                0 => 0,
                1..=33 => 1,
                34..=66 => 2,
                _ => 3,
            };
            let half = clip.intersect(&Rect::new(vx + 9 * s, cy - 8 * s, 7 * s, 16 * s));
            for i in 0..waves {
                draw::circle(fb, half, vx + 7 * s, cy, (3 + 2 * i) * s, Some(s), WHITE, 200);
            }
        }
        // The unread notifications: a dot and a count.
        if let Some(u) = l.unread {
            let u = u.scale(s);
            rounded_rect(fb, clip, u, u.height / 2, WHITE, rt::PILL_ALPHA);
            draw::circle(fb, clip, u.x + 9 * s, cy, 3 * s, None, WHITE, 200);
            let size = 11.0 * s as f32;
            let label = self.unread_label();
            self.text.draw(fb, clip, (u.x + 18 * s) as f32, baseline, u.right(), &label, size, WHITE, 191);
        }
    }

    /// A floating panel: Alpine's floating shadow, the bar colour, the
    /// outline.
    fn panel(&self, fb: &mut dyn Framebuffer, clip: Rect, panel: Rect, s: i32) {
        let radius = lt::RADIUS * s;
        const LAYERS: i32 = 4;
        for i in (1..=LAYERS).rev() {
            let grow = lt::SHADOW_SPREAD * s * i / LAYERS;
            let r = Rect::new(
                panel.x - grow,
                panel.y - grow + lt::SHADOW_OFFSET * s / 2,
                panel.width + 2 * grow,
                panel.height + 2 * grow,
            );
            rounded_rect(fb, clip, r, radius + grow, 0, (lt::SHADOW_ALPHA as i32 / (LAYERS + 2)) as u8);
        }
        rounded_rect(fb, clip, panel, radius, lt::BACKGROUND, 255);
        rounded_outline(fb, clip, panel, radius, WHITE, lt::OUTLINE_ALPHA);
    }

    fn draw_launcher(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        if !self.launcher.is_visible() {
            return;
        }
        let l = self.launcher_layout();
        let panel = l.panel.scale(s);
        if self.launcher_extent().scale(s).intersect(&clip).is_empty() {
            return;
        }
        self.panel(fb, clip, panel, s);

        // The search row: glyph, text or placeholder, caret.
        let search = l.search.scale(s);
        let cy = search.y + search.height / 2;
        let gx = search.x + lt::SEARCH_PADDING * s;
        let ring = 5 * s;
        draw::circle(fb, clip, gx + ring + s, cy - s, ring, Some((3 * s + 1) / 2), WHITE, lt::SEARCH_ICON_ALPHA);
        for i in 0..4 * s {
            let p = Rect::new(gx + ring + s + ring * 7 / 10 + i, cy - s + ring * 7 / 10 + i, 2 * s, 2 * s);
            fill(fb, clip, p, WHITE, lt::SEARCH_ICON_ALPHA);
        }
        let tx = gx + (lt::SEARCH_ICON + lt::SEARCH_GAP) * s;
        let size = lt::INPUT_SIZE * s as f32;
        let baseline = cy + self.text.cap_height(size) as i32 / 2;
        let right = search.right() - lt::SEARCH_PADDING * s;
        let query = self.launcher.query();
        let end = if query.is_empty() {
            self.text.draw(fb, clip, tx as f32, baseline, right, lt::PLACEHOLDER, size, WHITE, lt::PLACEHOLDER_ALPHA);
            tx as f32
        } else {
            self.text.draw(fb, clip, tx as f32, baseline, right, query, size, WHITE, 255)
        };
        let caret_h = 18 * s;
        fill(fb, clip, Rect::new(end as i32 + s, cy - caret_h / 2, s.max(1), caret_h), WHITE, 255);

        if query.is_empty() {
            return;
        }
        let w = panel.width;
        fill(fb, clip, Rect::new(panel.x, search.bottom(), w, s), WHITE, lt::DIVIDER_ALPHA);
        // The section title.
        let small = lt::SECTION_SIZE * s as f32;
        let sx = panel.x + (lt::LIST_PADDING + lt::ROW_PADDING) * s;
        let sb = search.bottom() + s + 8 * s + self.text.cap_height(small) as i32 + 2 * s;
        self.text.draw(fb, clip, sx as f32, sb, panel.right(), "Apps", small, WHITE, lt::SECTION_ALPHA);
        if l.empty {
            let row = l.first_row.scale(s);
            let b = row.y + row.height / 2 + self.text.cap_height(lt::TITLE_SIZE * s as f32) as i32 / 2;
            let msg = "No apps match";
            let x = (row.x + lt::ROW_PADDING * s) as f32;
            self.text.draw(fb, clip, x, b, row.right(), msg, lt::TITLE_SIZE * s as f32, WHITE, lt::SUBTITLE_ALPHA);
        }
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
            // The icon: a tile in a colour of the app's own with its initial.
            let ib = lt::ICON_BOX * s;
            let icon = Rect::new(row.x + lt::ROW_PADDING * s, row.y + (row.height - ib) / 2, ib, ib);
            self.tile(fb, clip, icon, &app.name, 15.0, s);
            // Name and description.
            let tx = (icon.right() + lt::ROW_ICON_GAP * s) as f32;
            let tsize = lt::TITLE_SIZE * s as f32;
            let right = row.right() - lt::ROW_PADDING * s;
            self.text.draw(fb, clip, tx, row.y + 21 * s, right, &app.name, tsize, WHITE, 255);
            let ssize = lt::SUBTITLE_SIZE * s as f32;
            self.text.draw(fb, clip, tx, row.y + 37 * s, right, &app.comment, ssize, WHITE, lt::SUBTITLE_ALPHA);
        }
        // The footer: what Enter and Escape do.
        if let Some(f) = l.footer {
            self.footer(fb, clip, f.scale(s), &[("Close", "Esc"), ("Open", "\u{21b5}")], s);
        }
    }

    /// A tile in a colour of `name`'s own with its initial, `size` px.
    fn tile(&self, fb: &mut dyn Framebuffer, clip: Rect, icon: Rect, name: &str, size: f32, s: i32) {
        rounded_rect(fb, clip, icon, (icon.width / 5).max(s), tile_colour(name), 255);
        let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        let isize = size * s as f32;
        let iw = self.text.width(&initial, isize) as i32;
        let ibase = icon.y + icon.height / 2 + self.text.cap_height(isize) as i32 / 2;
        self.text.draw(
            fb,
            clip,
            (icon.x + (icon.width - iw) / 2) as f32,
            ibase,
            icon.right(),
            &initial,
            isize,
            WHITE,
            255,
        );
    }

    /// A footer of `(label, key chip)` pairs, right-aligned (physical `f`).
    fn footer(&self, fb: &mut dyn Framebuffer, clip: Rect, f: Rect, pairs: &[(&str, &str)], s: i32) {
        fill(fb, clip, Rect::new(f.x, f.y - s, f.width, s), WHITE, lt::DIVIDER_ALPHA);
        let size = lt::FOOTER_SIZE * s as f32;
        let chip_size = 10.0 * s as f32;
        let base = f.y + f.height / 2 + self.text.cap_height(size) as i32 / 2;
        let mut x = f.right() - lt::SEARCH_PADDING * s;
        for (label, chip) in pairs {
            let cw = self.text.width(chip, chip_size) as i32 + 12 * s;
            let ch = 16 * s;
            x -= cw;
            let chip_rect = Rect::new(x, f.y + (f.height - ch) / 2, cw, ch);
            rounded_rect(fb, clip, chip_rect, 4 * s, WHITE, 26);
            let cb = chip_rect.y + ch / 2 + self.text.cap_height(chip_size) as i32 / 2;
            self.text.draw(fb, clip, (x + 6 * s) as f32, cb, chip_rect.right(), chip, chip_size, WHITE, 153);
            let lw = self.text.width(label, size) as i32;
            x -= 6 * s + lw;
            self.text.draw(fb, clip, x as f32, base, x + lw + s, label, size, WHITE, lt::SUBTITLE_ALPHA);
            x -= 12 * s;
        }
    }

    fn draw_toasts(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        let toasts = self.toast_layout();
        if toasts.is_empty() || self.toasts_extent().is_none_or(|e| e.scale(s).intersect(&clip).is_empty()) {
            return;
        }
        let size = nt::TEXT_SIZE * s as f32;
        for t in &toasts {
            let card = t.card.scale(s);
            self.panel(fb, clip, card, s);
            let tx = card.x + (nt::PADDING_X + nt::DOT + nt::DOT_GAP) * s;
            let right = card.right() - (nt::PADDING_X + nt::CLOSE + 6) * s;
            // The dot.
            let (dot, alpha) = if t.critical { (nt::CRITICAL, 255) } else { (WHITE, nt::DOT_ALPHA) };
            let dr = nt::DOT * s / 2;
            let dcx = card.x + nt::PADDING_X * s + dr;
            let dcy = card.y + (nt::PADDING_Y + nt::DOT_TOP) * s + dr;
            draw::circle(fb, clip, dcx, dcy, dr, None, dot, alpha);
            // The title and the message, a line every 16 pixels.
            let mut base = card.y + nt::PADDING_Y * s + (nt::LINE * s + self.text.cap_height(size) as i32) / 2;
            self.text.draw(fb, clip, tx as f32, base, right, &t.title, size, WHITE, nt::TITLE_ALPHA);
            for line in &t.lines {
                base += nt::LINE * s;
                self.text.draw(fb, clip, tx as f32, base, right, line, size, WHITE, nt::MESSAGE_ALPHA);
            }
            // The dismiss cross.
            let c = t.close.scale(s);
            let (cx, cy, arm) = (c.x + c.width / 2, c.y + c.height / 2, 3 * s);
            for i in -arm..=arm {
                fill(fb, clip, Rect::new(cx + i, cy + i, s.max(1), s.max(1)), WHITE, nt::CLOSE_ALPHA);
                fill(fb, clip, Rect::new(cx + i, cy - i, s.max(1), s.max(1)), WHITE, nt::CLOSE_ALPHA);
            }
            // Action buttons.
            let bsize = nt::BUTTON_SIZE * s as f32;
            for (b, _, label) in &t.buttons {
                let b = b.scale(s);
                rounded_rect(fb, clip, b, 6 * s, WHITE, nt::BUTTON_ALPHA);
                let bb = b.y + (b.height + self.text.cap_height(bsize) as i32) / 2;
                self.text.draw(fb, clip, (b.x + 10 * s) as f32, bb, b.right(), label, bsize, WHITE, 255);
            }
        }
    }

    fn draw_history(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        let Some(h) = self.history_layout() else { return };
        if shadowed(h.panel).scale(s).intersect(&clip).is_empty() {
            return;
        }
        let panel = h.panel.scale(s);
        self.panel(fb, clip, panel, s);
        // The header: a title and the clear chip.
        let tsize = lt::INPUT_SIZE * s as f32;
        let head = Rect::new(panel.x, panel.y, panel.width, lt::SEARCH_HEIGHT * s);
        let hb = head.y + (head.height + self.text.cap_height(tsize) as i32) / 2;
        let tx = (panel.x + lt::SEARCH_PADDING * s) as f32;
        self.text.draw(fb, clip, tx, hb, panel.right(), "Notifications", tsize, WHITE, 255);
        if let Some(c) = h.clear {
            let c = c.scale(s);
            rounded_rect(fb, clip, c, 6 * s, WHITE, 26);
            let csize = 11.0 * s as f32;
            let cb = c.y + (c.height + self.text.cap_height(csize) as i32) / 2;
            self.text.draw(fb, clip, (c.x + 8 * s) as f32, cb, c.right(), "Clear", csize, WHITE, 200);
        }
        fill(fb, clip, Rect::new(panel.x, head.bottom(), panel.width, s), WHITE, lt::DIVIDER_ALPHA);
        if h.rows.is_empty() {
            let size = lt::TITLE_SIZE * s as f32;
            let y = head.bottom() + s + lt::LIST_PADDING * s;
            let b = y + (lt::ROW_HEIGHT * s + self.text.cap_height(size) as i32) / 2;
            let x = (panel.x + (lt::LIST_PADDING + lt::ROW_PADDING) * s) as f32;
            self.text.draw(fb, clip, x, b, panel.right(), "No notifications", size, WHITE, lt::SUBTITLE_ALPHA);
            return;
        }
        let tsize = lt::TITLE_SIZE * s as f32;
        let ssize = lt::SUBTITLE_SIZE * s as f32;
        for (row, n) in h.rows.iter().zip(self.notes.history()) {
            let row = row.scale(s);
            let ib = lt::ICON_BOX * s;
            let icon = Rect::new(row.x + lt::ROW_PADDING * s, row.y + (row.height - ib) / 2, ib, ib);
            let who = if n.app_name.is_empty() { &n.summary } else { &n.app_name };
            self.tile(fb, clip, icon, who, 15.0, s);
            let tx = (icon.right() + lt::ROW_ICON_GAP * s) as f32;
            let time = self.age(n);
            let tw = self.text.width(&time, ssize) as i32;
            let right = row.right() - lt::ROW_PADDING * s;
            self.text.draw(fb, clip, tx, row.y + 21 * s, right - tw - 8 * s, &n.summary, tsize, WHITE, 255);
            let line = n.body.lines().next().unwrap_or("");
            self.text.draw(fb, clip, tx, row.y + 37 * s, right, line, ssize, WHITE, lt::SUBTITLE_ALPHA);
            self.text.draw(
                fb,
                clip,
                (right - tw) as f32,
                row.y + 21 * s,
                right,
                &time,
                ssize,
                WHITE,
                lt::SUBTITLE_ALPHA,
            );
        }
    }

    /// How long ago a notification came, briefly.
    fn age(&self, n: &Notification) -> String {
        let secs = self.now_ms.saturating_sub(n.arrived_ms) / 1000;
        match secs {
            0..60 => "now".into(),
            60..3600 => alloc::format!("{} min", secs / 60),
            _ => alloc::format!("{} h", secs / 3600),
        }
    }

    fn draw_switcher(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        let Some(extent) = self.switcher_extent() else { return };
        if extent.scale(s).intersect(&clip).is_empty() {
            return;
        }
        let (panel, cards) = self.switcher.layout(self.width, self.height);
        self.panel(fb, clip, panel.scale(s), s);
        let size = sw::TEXT_SIZE * s as f32;
        for (i, (card, e)) in cards.iter().zip(self.switcher.entries()).enumerate() {
            let c = card.scale(s);
            let r = sw::RADIUS * s;
            rounded_rect(fb, clip, c, r, WHITE, sw::CARD_ALPHA);
            if i == self.switcher.selected() {
                rounded_outline(fb, clip, c, r, WHITE, sw::SELECTED_ALPHA);
                let inner = Rect::new(c.x + s, c.y + s, c.width - 2 * s, c.height - 2 * s);
                rounded_outline(fb, clip, inner, r - s, WHITE, sw::SELECTED_ALPHA);
            } else {
                rounded_outline(fb, clip, c, r, WHITE, sw::RING_ALPHA);
            }
            // The name row: an icon tile and the title.
            let icon = Rect::new(c.x + 6 * s, c.y + 6 * s, sw::ICON * s, sw::ICON * s);
            let who = &e.title;
            self.tile(fb, clip, icon, who, 10.0, s);
            let base = c.y + (sw::HEADER * s + self.text.cap_height(size) as i32) / 2;
            let alpha = if e.minimized { 153 } else { 255 };
            self.text.draw(
                fb,
                clip,
                (icon.right() + 8 * s) as f32,
                base,
                c.right() - 6 * s,
                &e.title,
                size,
                WHITE,
                alpha,
            );
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

/// A floating panel's rectangle with its shadow.
fn shadowed(p: Rect) -> Rect {
    let s = lt::SHADOW_SPREAD;
    Rect::new(p.x - s, p.y - s, p.width + 2 * s, p.height + 2 * s + lt::SHADOW_OFFSET)
}

/// A tile colour for an app, from its name: muted, light enough for white
/// text.
fn tile_colour(name: &str) -> u32 {
    const COLOURS: [u32; 6] = [0x3b_6e_a8, 0x4f_8a_5b, 0x8a_5a_a8, 0xa8_6a_3b, 0x3b_8a_8a, 0xa8_45_5a];
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    COLOURS[(h % COLOURS.len() as u32) as usize]
}

/// The layer under the windows.
#[derive(Debug)]
pub struct Below<'a>(&'a Shell);

/// The layer over the windows.
#[derive(Debug)]
pub struct Above<'a>(&'a Shell);

impl Layer for Below<'_> {
    fn draw(&self, fb: &mut dyn Framebuffer, clip: Rect, scale: i32) {
        self.0.draw_wallpaper(fb, clip, scale);
        self.0.draw_ridge(fb, clip, scale);
    }
}

impl Layer for Above<'_> {
    fn draw(&self, fb: &mut dyn Framebuffer, clip: Rect, scale: i32) {
        self.0.draw_snap(fb, clip, scale);
        self.0.draw_toasts(fb, clip, scale);
        self.0.draw_history(fb, clip, scale);
        self.0.draw_switcher(fb, clip, scale);
        self.0.draw_launcher(fb, clip, scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::builtins;
    use crate::switcher::Thumb;
    use poly_dbus::notifications::reason;
    use poly_wayland::compose::MemoryFramebuffer;

    fn shell() -> Shell {
        let mut s = Shell::new(1280, 800, builtins(), Text::new(None));
        s.set_time(1_793_491_200, 0);
        s
    }

    fn render(s: &Shell) -> MemoryFramebuffer {
        let mut fb = MemoryFramebuffer::new(1280, 800);
        let all = Rect::new(0, 0, 1280, 800);
        s.below().draw(&mut fb, all, 1);
        s.above().draw(&mut fb, all, 1);
        fb
    }

    fn note(summary: &str, body: &str) -> Notify {
        Notify {
            app_name: "test".into(),
            summary: summary.into(),
            body: body.into(),
            expire_timeout: -1,
            ..Notify::default()
        }
    }

    #[test]
    fn the_ridge_and_the_launcher_on_an_empty_desktop() {
        let s = shell();
        assert_eq!(s.clock(), "00:00 Nov 1");
        assert!(s.launcher().is_visible(), "shown on an empty desktop");
        let fb = render(&s);
        // The ridge: rgb(32, 32, 32) across the top, 24 rows.
        assert_eq!(fb.pixel(640, 2), Some(0x20_20_20));
        assert_eq!(fb.pixel(640, 23), Some(0x20_20_20));
        assert_ne!(fb.pixel(640, 24), Some(0x20_20_20));
        // The launcher: 720 wide, centred: x 280..1000; its search row 48
        // high at (800 - 48) / 2 - 80 = 296.
        let l = s.launcher_layout();
        assert_eq!(l.panel, Rect::new(280, 296, 720, 48));
        assert_eq!(fb.pixel(600, 320), Some(0x20_20_20), "the panel");
        assert_eq!(fb.pixel(281, 297).map(|p| p == 0x20_20_20), Some(false), "a rounded corner");
        // The clock's text is drawn in the ridge's right part.
        let lit = (1100..1276).any(|x| (4..20).any(|y| fb.pixel(x, y) == Some(0xff_ffff)));
        assert!(lit);
    }

    #[test]
    fn typing_lists_apps_and_enter_launches() {
        let mut s = shell();
        s.take_damage();
        for c in "demo".chars() {
            assert_eq!(s.key(Key::Char(c)), None);
        }
        let damage = s.take_damage();
        assert!(!damage.is_empty(), "typing damages the launcher");
        assert!(damage.iter().all(|d| d.y >= 296 - lt::SHADOW_SPREAD));
        let l = s.launcher_layout();
        assert_eq!(l.rows, 1);
        assert!(l.footer.is_some());
        assert_eq!(l.panel.height, 48 + 1 + 28 + 48 + 6 + 1 + 36);
        // Clicking the row starts the app; the launcher goes.
        let row = s.row_rect(&l, 0);
        match s.press(row.x + 5, row.y + 5) {
            Press::Launch(app) => assert_eq!(app.name, "Poly Demo"),
            p => panic!("{p:?}"),
        }
        assert!(!s.launcher().is_visible());
        // Hidden: presses outside the ridge are the windows'.
        assert_eq!(s.press(640, 500), Press::NotMine);
        assert_eq!(s.press(640, 10), Press::Taken);
        // The hotkey brings it back, empty; a press outside hides it.
        s.toggle_launcher();
        assert!(s.launcher().is_visible());
        assert_eq!(s.launcher().query(), "");
        assert_eq!(s.press(10, 700), Press::Taken);
        assert!(!s.launcher().is_visible());
        // Enter launches too.
        s.toggle_launcher();
        for c in "wayland".chars() {
            s.key(Key::Char(c));
        }
        assert_eq!(s.key(Key::Enter).map(|a| a.name), Some("Wayland Test Client".into()));
    }

    #[test]
    fn no_match_and_the_clock() {
        let mut s = shell();
        s.key(Key::Char('q'));
        s.key(Key::Char('q'));
        s.key(Key::Char('q'));
        let l = s.launcher_layout();
        assert!(l.empty);
        assert_eq!(s.key(Key::Enter), None);
        s.take_damage();
        assert!(!s.set_time(1_793_491_210, 0), "the same minute");
        assert!(s.set_time(1_793_491_260, 0));
        assert_eq!(s.take_damage(), [Rect::new(0, 0, 1280, 24)]);
        // A small screen: fewer rows fit.
        let small = Shell::new(640, 400, builtins(), Text::new(None));
        assert!(small.room_for_rows() < lt::MAX_ROWS);
        assert!(small.room_for_rows() >= 1);
    }

    #[test]
    fn toasts_show_stack_and_go() {
        let mut s = shell();
        s.hide_launcher();
        s.set_now_ms(1000);
        s.take_damage();
        let a = s.notify(note("Hello", "From a test client"));
        let damage = s.take_damage();
        // The toast's card and the ridge (the unread count) are damaged.
        assert!(damage.iter().any(|d| d.right() >= 1280 - 8 && d.y <= 32 + 12));
        assert!(damage.contains(&s.ridge_rect()));
        let t = s.toast_layout();
        assert_eq!(t[0].card, Rect::new(1280 - 8 - 360, 24 + 8, 360, 10 + 16 + 16 + 10));
        let fb = render(&s);
        assert_eq!(fb.pixel(1280 - 8 - 180, 24 + 8 + 40), Some(0x20_20_20), "the card's colour");
        // The unread count shows on the ridge.
        assert!(s.ridge_layout().unread.is_some());
        let b = s.notify(note("Second", ""));
        let t = s.toast_layout();
        assert_eq!((t[0].id, t[1].id), (b, a), "newest on top");
        assert_eq!(t[1].card.y, t[0].card.bottom() + 8);
        // A click dismisses the newest; the other moves up.
        let c = t[0].card;
        assert_eq!(s.press(c.x + 40, c.y + 10), Press::Taken);
        assert_eq!(s.take_closed_notifications(), [(b, reason::DISMISSED)]);
        assert_eq!(s.toast_layout()[0].card.y, 32);
        // The other expires after the default 7 s.
        s.set_now_ms(1000 + 7000);
        assert_eq!(s.take_closed_notifications(), [(a, reason::EXPIRED)]);
        assert!(s.toast_layout().is_empty());
        // The history keeps both; opening it marks them read.
        let u = s.ridge_layout().unread.unwrap();
        s.press(u.x + 2, u.y + 2);
        assert!(s.history_open());
        assert!(s.ridge_layout().unread.is_none());
        let h = s.history_layout().unwrap();
        assert_eq!(h.rows.len(), 2);
        let fb = render(&s);
        assert_eq!(fb.pixel((h.panel.x + 200) as u32, (h.panel.y + 30) as u32), Some(0x20_20_20));
        // Escape closes it; a press outside would too.
        assert!(s.overlay_key(Key::Escape));
        assert!(!s.history_open());
    }

    #[test]
    fn actions_and_long_messages() {
        let mut s = shell();
        s.hide_launcher();
        let mut n =
            note("Update ready", "A rather long message that will certainly not fit on one single line of the toast");
        n.actions = alloc::vec![("default".into(), "Open".into()), ("later".into(), "Later".into())];
        let id = s.notify(n);
        let t = &s.toast_layout()[0];
        assert!(t.lines.len() >= 2, "wrapped: {:?}", t.lines);
        assert_eq!(t.buttons.len(), 1, "the default action has no button");
        let b = t.buttons[0].0;
        s.press(b.x + 2, b.y + 2);
        assert_eq!(s.take_invoked_actions(), [(id, "later".into())]);
        assert_eq!(s.take_closed_notifications(), [(id, reason::DISMISSED)]);
    }

    #[test]
    fn the_switcher_and_the_snap_preview() {
        let mut s = shell();
        s.hide_launcher();
        let entry = |id: u32| Entry {
            window: WindowId(id),
            title: alloc::format!("Window {id}"),
            app_id: String::new(),
            minimized: id == 3,
            thumb: Thumb { width: 160, height: 100, pixels: alloc::vec![0x30_80_50; 160 * 100] },
        };
        s.take_damage();
        s.open_switcher(alloc::vec![entry(1), entry(2), entry(3)], false);
        assert!(s.switcher_open());
        assert!(!s.take_damage().is_empty());
        let fb = render(&s);
        let (panel, cards) = s.switcher().layout(1280, 800);
        assert_eq!(fb.pixel((panel.x + 5) as u32, (panel.y + panel.height / 2) as u32), Some(0x20_20_20));
        let c = cards[1];
        assert_eq!(fb.pixel((c.x + c.width / 2) as u32, (c.y + 28 + 62) as u32), Some(0x30_80_50), "a thumbnail");
        s.switcher_step(true);
        assert_eq!(s.switcher_commit(), Some(WindowId(3)));
        // A click on a card switches too.
        s.open_switcher(alloc::vec![entry(1), entry(2)], false);
        let (_, cards) = s.switcher().layout(1280, 800);
        assert_eq!(s.press(cards[0].x + 3, cards[0].y + 3), Press::Switch(WindowId(1)));
        // The snap preview damages where it was and is.
        s.take_damage();
        let left = Rect::new(4, 28, 632, 768);
        s.set_snap_preview(Some(left));
        assert_eq!(s.take_damage(), [left]);
        let fb = render(&s);
        assert_ne!(fb.pixel(300, 400), render(&shell_without_launcher()).pixel(300, 400), "drawn over the wallpaper");
        s.set_snap_preview(None);
        assert_eq!(s.take_damage(), [left]);
    }

    fn shell_without_launcher() -> Shell {
        let mut s = shell();
        s.hide_launcher();
        s
    }

    #[test]
    fn status_and_settings_from_the_ridge() {
        let mut s = shell();
        s.hide_launcher();
        s.take_damage();
        assert!(s.set_network(Network::Online("eth0".into())));
        assert!(!s.set_network(Network::Online("eth0".into())));
        assert!(s.set_volume(Some((60, false))));
        assert_eq!(s.take_damage(), [s.ridge_rect(), s.ridge_rect()]);
        let st = s.ridge_layout().status;
        assert_eq!(s.press(st.x + 3, st.y + 3), Press::Settings(Page::Sound));
        // Settings: the wallpaper repaints everything; the time zone moves the clock.
        assert!(s.apply(&Change::Wallpaper("forest".into())));
        assert!(s.take_damage().contains(&Rect::new(0, 0, 1280, 800)));
        assert!(s.apply(&Change::TimeZone("Europe/Zurich".into())));
        let t = 1_793_491_200;
        assert!(s.set_time(t, s.local_offset(t)));
        assert_eq!(s.clock(), "01:00 Nov 1");
        assert_eq!(s.local_time(t), "01:00 Nov 1 CET");
    }
}
