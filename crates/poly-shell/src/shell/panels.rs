//! The panels that open from the ridge and the notices, as Alpine Shell
//! draws them (ADR 0126):
//!
//! - the **system menu** under the dashed circle (`systemMenu.tsx`):
//!   Restart and Shut Down, each asking to be confirmed by a second click;
//! - the **control centre** under the status pill (`status/controlCenter.tsx`,
//!   `controlSlider.tsx`, `soundMenu.tsx`): the network and the sound card
//!   (each opens a list), the volume slider whose icon mutes, a button to
//!   the settings;
//! - the **calendar** under the clock (`calendar/calendarFlyout.tsx`): one
//!   day, its hours, the line of the time now, days before and after;
//! - the **notification history** under the unread count in the middle
//!   (Poly's; Alpine keeps no history), in the style of its lists;
//! - the **notices** (`noticeStrip.tsx`) at the top right, newest first.
//!
//! Layouts are logical, from the panel's body at full height; drawing and
//! hit-testing use the same layout.

use super::{Power, Press, Shell, WHITE};
use crate::chrome::Geometry;
use crate::draw::{Face, Style, fill, rounded_rect};
use crate::icons::{Icon, lucide};
use crate::settings::Change;
use crate::settings_ui::Page;
use crate::theme::{ink, notice as nt, panel as pt, ridge as rt};
use alloc::string::String;
use alloc::vec::Vec;
use poly_dbus::notifications::Urgency;
use poly_wayland::compose::{Framebuffer, Rect};

/// The panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    /// The system menu.
    Menu,
    /// The control centre.
    Control,
    Calendar,
    /// The notification history.
    History,
}

/// What the control centre shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlView {
    #[default]
    Main,
    Network,
    Sound,
}

/// What the pointer is over in a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    MenuRow(usize),
    Item(ControlView),
    Slider,
    Mute,
    Gear,
    Back,
    Row(usize),
    Prev,
    Next,
    Today,
    Clear,
}

/// The open panel's state.
#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pub view: ControlView,
    pub hover: Option<Hit>,
    pub dragging: bool,
    /// The system menu's action waiting for its confirming click.
    pub armed: Option<Power>,
    /// The calendar's day, relative to today (-7 to 30, as Alpine).
    pub day: i32,
}

const MENU: [(Power, &str, &str, Icon); 2] = [
    (Power::Restart, "Restart", "Confirm restart?", lucide::ROTATE_CCW),
    (Power::ShutDown, "Shut Down", "Confirm shut down?", lucide::POWER),
];

const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A notice in the strip, logical.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NoticeBox {
    pub id: u32,
    pub rect: Rect,
    pub close: Rect,
    pub title: String,
    pub lines: Vec<String>,
    /// (button, action key, label).
    pub buttons: Vec<(Rect, String, String)>,
    pub critical: bool,
}

/// A row of a list in a panel: its rectangle, its text, a check.
pub(super) struct ListRow {
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) checked: bool,
    pub(super) dim: bool,
}

/// The control centre's sub view: the header and groups of rows.
pub(super) struct SubView {
    pub(super) back: Rect,
    pub(super) title: &'static str,
    /// (title's baseline y, rows).
    pub(super) groups: Vec<(&'static str, i32, Vec<ListRow>)>,
    pub(super) height: i32,
}

impl Shell {
    // -----------------------------------------------------------------
    // Geometry.

    pub(super) fn panel_geometry(&self, p: Panel) -> Geometry {
        let r = self.ridge_layout();
        let (w, h) = self.panel_size(p);
        let trigger = self.trigger_rect(&r, p);
        let centre = if trigger.is_empty() { self.width / 2 } else { trigger.x + trigger.width / 2 };
        Geometry::place(centre, w, h, self.width)
    }

    pub(super) fn panel_size(&self, p: Panel) -> (i32, i32) {
        match p {
            Panel::Menu => {
                let st = Style::new(Face::Sans, pt::MENU_SIZE);
                let label =
                    MENU.iter().map(|m| self.text.measure(m.2, st).max(self.text.measure(m.1, st)) as i32).max();
                let w = 2 * pt::MENU_PADDING + 8 + pt::MENU_ICON + 10 + label.unwrap_or(0) + 8;
                (w.max(pt::MENU_MIN_WIDTH), 2 * pt::MENU_PADDING + MENU.len() as i32 * pt::MENU_ROW)
            }
            Panel::Control => {
                let h = match self.panels.view {
                    ControlView::Main => self.control_main(Rect::new(0, 0, pt::CONTROL_WIDTH, 0)).height,
                    v => self.sub_view(v, Rect::new(0, 0, pt::CONTROL_WIDTH, 0)).height,
                };
                (pt::CONTROL_WIDTH, h)
            }
            Panel::Calendar => {
                (pt::CAL_GUTTER + pt::CAL_COLUMN, pt::CAL_HEADER + pt::CAL_DAY + pt::CAL_ALL_DAY + pt::CAL_SCROLL)
            }
            Panel::History => {
                let n = self.notes.history_len().min(crate::theme::launcher::MAX_ROWS) as i32;
                let rows = if n == 0 { 32 } else { n * 46 - 2 };
                (pt::HISTORY_WIDTH, 41 + 6 + rows + 6)
            }
        }
    }

    /// The control centre's main view in `body`: (items, slider, mute
    /// icon, track, gear, height).
    pub(super) fn control_main(&self, body: Rect) -> ControlMain {
        let x0 = body.x + pt::CONTROL_PADDING;
        let w = body.width - 2 * pt::CONTROL_PADDING;
        let mut y = body.y + pt::CONTROL_PADDING;
        let mut items = Vec::new();
        for view in [ControlView::Network, ControlView::Sound] {
            let h = if self.item_value(view).is_some() { pt::ITEM_HEIGHT } else { 19 };
            items.push((view, Rect::new(x0, y, w, h)));
            y += h + pt::CONTROL_GAP;
        }
        y += 4;
        let slider = Rect::new(x0, y, w, pt::SLIDER_HEIGHT);
        let icon = Rect::new(x0 + 12, y + (pt::SLIDER_HEIGHT - 15) / 2, 15, 15);
        let tx = icon.right() + 12;
        let track =
            Rect::new(tx, y + (pt::SLIDER_HEIGHT - pt::SLIDER_TRACK) / 2, slider.right() - 12 - tx, pt::SLIDER_TRACK);
        y += pt::SLIDER_HEIGHT + pt::CONTROL_GAP;
        let gear = Rect::new(x0, y, pt::BUTTON, pt::BUTTON);
        y += pt::BUTTON + pt::CONTROL_PADDING;
        ControlMain { items, slider, icon, track, gear, height: y - body.y }
    }

    /// What an item of the control centre shows under its label.
    fn item_value(&self, view: ControlView) -> Option<String> {
        match view {
            ControlView::Network => match &self.network {
                super::Network::Online(i) => Some(i.clone()),
                _ => None,
            },
            ControlView::Sound => self.sound_card.clone(),
            ControlView::Main => None,
        }
    }

    pub(super) fn sub_view(&self, v: ControlView, body: Rect) -> SubView {
        let x0 = body.x + pt::CONTROL_PADDING;
        let w = body.width - 2 * pt::CONTROL_PADDING;
        let mut y = body.y + pt::CONTROL_PADDING;
        let title = if v == ControlView::Sound { "Sound" } else { "Network" };
        let back = Rect::new(x0, y, w / 2, 20);
        y += 20 + 4 + 8;
        let row = |y: i32, label: String, checked: bool, dim: bool| ListRow {
            rect: Rect::new(x0, y, w, 32),
            label,
            checked,
            dim,
        };
        let lists: Vec<(&'static str, Vec<Choice>)> = match v {
            ControlView::Sound => alloc::vec![
                (
                    "Output",
                    match &self.sound_card {
                        Some(c) => alloc::vec![(c.clone(), true, false)],
                        None => alloc::vec![("None connected".into(), false, true)],
                    }
                ),
                ("Input", alloc::vec![("None connected".into(), false, true)]),
            ],
            _ => alloc::vec![(
                "Ethernet",
                match &self.network {
                    super::Network::Online(i) => alloc::vec![(alloc::format!("{i} \u{b7} Connected"), true, false)],
                    _ => alloc::vec![("Not connected".into(), false, true)],
                }
            )],
        };
        let mut groups = Vec::new();
        for (name, items) in lists {
            let tb = y + 12;
            y += 17 + 2;
            let mut rows = Vec::new();
            for (label, checked, dim) in items {
                rows.push(row(y, label, checked, dim));
                y += 32 + 2;
            }
            y += 8 - 2;
            groups.push((name, tb, rows));
        }
        y += pt::CONTROL_PADDING - 8;
        SubView { back, title, groups, height: y - body.y }
    }

    // -----------------------------------------------------------------
    // Input.

    /// The thing at (`x`, `y`) in panel `p` (open, at full height).
    fn panel_hit(&self, p: Panel, x: i32, y: i32) -> Option<Hit> {
        let body = self.panel_geometry(p).body(rt::HEIGHT);
        match p {
            Panel::Menu => (0..MENU.len()).find(|&i| self.menu_row(body, i).contains(x, y)).map(Hit::MenuRow),
            Panel::Control => match self.panels.view {
                ControlView::Main => {
                    let m = self.control_main(body);
                    if let Some((v, _)) = m.items.iter().find(|(_, r)| r.contains(x, y)) {
                        return Some(Hit::Item(*v));
                    }
                    if m.icon.grow(4).contains(x, y) {
                        return Some(Hit::Mute);
                    }
                    if m.slider.contains(x, y) {
                        return Some(Hit::Slider);
                    }
                    m.gear.contains(x, y).then_some(Hit::Gear)
                }
                v => {
                    let sv = self.sub_view(v, body);
                    if sv.back.contains(x, y) {
                        return Some(Hit::Back);
                    }
                    let rows = sv.groups.iter().flat_map(|g| g.2.iter());
                    rows.enumerate().find(|(_, r)| r.rect.contains(x, y)).map(|(i, _)| Hit::Row(i))
                }
            },
            Panel::Calendar => {
                let c = self.calendar_layout(body);
                [(c.prev, Hit::Prev), (c.next, Hit::Next), (c.today.unwrap_or_default(), Hit::Today)]
                    .into_iter()
                    .find(|(r, _)| r.contains(x, y))
                    .map(|(_, h)| h)
            }
            Panel::History => {
                let h = self.history_layout(body);
                if h.clear.is_some_and(|c| c.contains(x, y)) {
                    return Some(Hit::Clear);
                }
                h.rows.iter().position(|r| r.contains(x, y)).map(Hit::Row)
            }
        }
    }

    pub(super) fn panel_hover(&mut self, x: i32, y: i32) {
        let Some(p) = self.panel else { return };
        let hit = self.panel_hit(p, x, y);
        if hit != self.panels.hover {
            self.panels.hover = hit;
            self.touch();
        }
    }

    /// A press in the open panel `p`.
    pub(super) fn panel_press(&mut self, p: Panel, x: i32, y: i32) -> Press {
        let hit = self.panel_hit(p, x, y);
        match (p, hit) {
            (Panel::Menu, Some(Hit::MenuRow(i))) => {
                let action = MENU[i].0;
                if self.panels.armed == Some(action) {
                    self.close_panel();
                    return Press::Power(action);
                }
                self.panels.armed = Some(action);
                self.touch();
            }
            (Panel::Control, Some(Hit::Item(v))) => {
                self.panels.view = v;
                self.panels.hover = None;
                self.touch();
            }
            (Panel::Control, Some(Hit::Back)) => {
                self.panels.view = ControlView::Main;
                self.panels.hover = None;
                self.touch();
            }
            (Panel::Control, Some(Hit::Slider)) if self.volume.is_some() => {
                self.panels.dragging = true;
                self.panel_drag(x, y);
            }
            (Panel::Control, Some(Hit::Mute)) => {
                if let Some((_, m)) = self.volume {
                    self.changes.push(Change::Muted(!m));
                }
            }
            (Panel::Control, Some(Hit::Gear)) => {
                self.close_panel();
                return Press::Settings(Page::Display);
            }
            (Panel::Calendar, Some(h @ (Hit::Prev | Hit::Next | Hit::Today))) => {
                self.panels.day = match h {
                    Hit::Prev => (self.panels.day - 1).max(-7),
                    Hit::Next => (self.panels.day + 1).min(30),
                    _ => 0,
                };
                self.touch();
            }
            (Panel::History, Some(Hit::Clear)) => {
                self.notes_change(|s| s.notes.clear_history());
                self.touch();
            }
            _ => {}
        }
        Press::Taken
    }

    /// The volume slider dragged to `x`.
    pub(super) fn panel_drag(&mut self, x: i32, _y: i32) {
        let body = self.panel_geometry(Panel::Control).body(rt::HEIGHT);
        let t = self.control_main(body).track;
        let v = (((x - t.x) * 100 + t.width / 2) / t.width.max(1)).clamp(0, 100) as u32;
        if self.volume.is_some_and(|(cur, _)| cur != v) && self.changes.last() != Some(&Change::Volume(v)) {
            self.changes.push(Change::Volume(v));
        }
    }

    /// A press on the notices: buttons invoke actions, the cross dismisses,
    /// the rest invokes the default action (or dismisses).
    pub(super) fn notices_press(&mut self, x: i32, y: i32) -> Option<Press> {
        let n = self.notice_layout(rt::HEIGHT).into_iter().find(|n| n.rect.contains(x, y))?;
        let now = self.now_ms;
        let id = n.id;
        if let Some((_, key, _)) = n.buttons.iter().find(|b| b.0.contains(x, y)) {
            let key = key.clone();
            self.notes_change(|s| s.notes.invoke(id, &key, now));
        } else if !n.close.contains(x, y) && self.notes.has_action(id, "default") {
            self.notes_change(|s| s.notes.invoke(id, "default", now));
        } else {
            self.notes_change(|s| s.notes.dismiss(id, now));
        }
        Some(Press::Taken)
    }

    // -----------------------------------------------------------------
    // The notices.

    pub(super) fn notice_width(&self) -> i32 {
        nt::WIDTH.min(self.width - 16)
    }

    /// The notices hanging from the ridge at the top right, newest first,
    /// their body's top at `top`.
    pub(super) fn notice_layout(&self, top: i32) -> Vec<NoticeBox> {
        let w = self.notice_width();
        let x = self.width - w;
        let mut y = top;
        let text_x = nt::PADDING_X + nt::DOT + nt::DOT_GAP;
        let close_w = 13 + 12;
        let text_w = (w - text_x - nt::PADDING_X - close_w + 4) as f32;
        let title_st = Style::new(Face::Medium, nt::TEXT_SIZE);
        let mut out = Vec::new();
        for (i, n) in self.notes.toasts().into_iter().enumerate() {
            if i > 0 {
                y += 1;
            }
            let (title, body) =
                if n.summary.is_empty() { (n.app_name.clone(), &n.body) } else { (n.summary.clone(), &n.body) };
            let title = self.text.fit_style(&title, title_st, text_w);
            let lines =
                if body.is_empty() { Vec::new() } else { self.text.wrap(body, nt::TEXT_SIZE, text_w, nt::MAX_LINES) };
            let actions: Vec<&(String, String)> = n.actions.iter().filter(|a| a.0 != "default").take(2).collect();
            let mut h = 2 * nt::PADDING_Y + nt::LINE * (1 + lines.len() as i32);
            if !actions.is_empty() {
                h += 8 + nt::BUTTON_HEIGHT;
            }
            let rect = Rect::new(x, y, w, h);
            let close = Rect::new(rect.right() - nt::PADDING_X + 4 - close_w, y + nt::PADDING_Y - 2, close_w, 20);
            let mut bx = x + text_x;
            let by = rect.bottom() - nt::PADDING_Y - nt::BUTTON_HEIGHT;
            let mut buttons = Vec::new();
            for (key, label) in actions {
                let bw = self.text.width(label, nt::BUTTON_SIZE) as i32 + 16;
                buttons.push((Rect::new(bx, by, bw, nt::BUTTON_HEIGHT), key.clone(), label.clone()));
                bx += bw + 6;
            }
            out.push(NoticeBox {
                id: n.id,
                rect,
                close,
                title,
                lines,
                buttons,
                critical: n.urgency == Urgency::Critical,
            });
            y += h;
            if y > self.height - 40 {
                break;
            }
        }
        out
    }

    pub(super) fn draw_notices(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32, body: Rect) {
        let st = Style::new(Face::Sans, nt::TEXT_SIZE).scaled(s);
        let title_st = Style::new(Face::Medium, nt::TEXT_SIZE).scaled(s);
        let fs = s as f32;
        for (i, n) in self.notice_layout(body.y).iter().enumerate() {
            let r = n.rect.scale(s);
            if i > 0 {
                fill(
                    fb,
                    clip,
                    Rect::new(r.x + nt::PADDING_X * s, r.y - s, r.width - 2 * nt::PADDING_X * s, s),
                    WHITE,
                    nt::DIVIDER_ALPHA,
                );
            }
            // The dot.
            let (dot, alpha) = if n.critical { (nt::CRITICAL, 255) } else { (WHITE, nt::DOT_ALPHA) };
            let d = nt::DOT as f32 * fs;
            let dcx = r.x as f32 + nt::PADDING_X as f32 * fs + d / 2.0;
            let dcy = r.y as f32 + (nt::PADDING_Y + nt::DOT_TOP) as f32 * fs + d / 2.0;
            poly_wayland::compose::paint::circle(fb, clip, dcx, dcy, d / 2.0, None, dot, alpha);
            // The title and the message, a line every 16 pixels.
            let tx = (r.x + (nt::PADDING_X + nt::DOT + nt::DOT_GAP) * s) as f32;
            let right = n.close.scale(s).x;
            let mut top = r.y + nt::PADDING_Y * s;
            let b = self.text.centre(top, nt::LINE * s, title_st);
            self.text.draw_style(fb, clip, tx, b, right, &n.title, title_st, WHITE, nt::TITLE_ALPHA);
            for line in &n.lines {
                top += nt::LINE * s;
                let b = self.text.centre(top, nt::LINE * s, st);
                self.text.draw_style(fb, clip, tx, b, r.right(), line, st, WHITE, nt::MESSAGE_ALPHA);
            }
            // The dismiss cross, a 13-pixel ×.
            let c = n.close.scale(s);
            let xst = Style::new(Face::Sans, 13.0).scaled(s);
            let xw = self.text.measure("\u{d7}", xst);
            let xb = self.text.centre(c.y, c.height, xst);
            let xx = c.x as f32 + (c.width as f32 - xw) / 2.0;
            self.text.draw_style(fb, clip, xx, xb, c.right(), "\u{d7}", xst, WHITE, nt::CLOSE_ALPHA);
            // Action buttons.
            let bst = Style::new(Face::Sans, nt::BUTTON_SIZE).scaled(s);
            for (b, _, label) in &n.buttons {
                let b = b.scale(s);
                rounded_rect(fb, clip, b, 6 * s, WHITE, nt::BUTTON_ALPHA);
                let bb = self.text.centre(b.y, b.height, bst);
                self.text.draw_style(fb, clip, (b.x + 8 * s) as f32, bb, b.right(), label, bst, WHITE, 255);
            }
        }
    }

    // -----------------------------------------------------------------
    // Drawing the panels.

    pub(super) fn draw_panel(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32, p: Panel, body: Rect) {
        match p {
            Panel::Menu => self.draw_menu(fb, clip, s, body),
            Panel::Control => match self.panels.view {
                ControlView::Main => self.draw_control(fb, clip, s, body),
                v => self.draw_sub_view(fb, clip, s, v, body),
            },
            Panel::Calendar => self.draw_calendar(fb, clip, s, body),
            Panel::History => self.draw_history(fb, clip, s, body),
        }
    }

    pub(super) fn menu_row(&self, body: Rect, i: usize) -> Rect {
        Rect::new(
            body.x + pt::MENU_PADDING,
            body.y + pt::MENU_PADDING + i as i32 * pt::MENU_ROW,
            body.width - 2 * pt::MENU_PADDING,
            pt::MENU_ROW,
        )
    }

    fn draw_menu(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32, body: Rect) {
        let st = Style::new(Face::Sans, pt::MENU_SIZE).scaled(s);
        for (i, (action, label, confirm, icon)) in MENU.iter().enumerate() {
            let r = self.menu_row(body, i).scale(s);
            let hover = self.panels.hover == Some(Hit::MenuRow(i));
            if hover {
                rounded_rect(fb, clip, r, 6 * s, WHITE, ink::SELECTED);
            }
            let a = if hover { 255 } else { pt::MENU_ALPHA };
            let isz = pt::MENU_ICON * s;
            let ix = r.x + 8 * s;
            icon.draw(fb, clip, ix as f32, (r.y + (r.height - isz) / 2) as f32, isz as f32, WHITE, a);
            let text = if self.panels.armed == Some(*action) { confirm } else { label };
            let b = self.text.centre(r.y, r.height, st);
            self.text.draw_style(fb, clip, (ix + isz + 10 * s) as f32, b, r.right(), text, st, WHITE, a);
        }
    }

    fn draw_control(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32, body: Rect) {
        let m = self.control_main(body);
        let label_st = Style::new(Face::Sans, 12.0).scaled(s);
        let value_st = Style::new(Face::Sans, 13.0).scaled(s);
        for (view, r) in &m.items {
            let r = r.scale(s);
            let value = self.item_value(*view);
            let active = match view {
                ControlView::Network => matches!(self.network, super::Network::Online(_)),
                _ => self.volume.is_some(),
            };
            let a = if active { 255 } else { pt::DIM_ALPHA };
            let icon = if *view == ControlView::Network { lucide::ETHERNET_PORT } else { lucide::SPEAKER };
            let isz = pt::ITEM_ICON * s;
            let ix = r.x + 4 * s;
            icon.draw(fb, clip, ix as f32, (r.y + (r.height - isz) / 2) as f32, isz as f32, WHITE, a);
            let tx = (ix + isz + 12 * s) as f32;
            let name = if *view == ControlView::Network { "Ethernet" } else { "Sound" };
            match value {
                Some(v) => {
                    // 12 px over 13 px at 60 %, the second 2 pixels up.
                    let b1 = r.y + 12 * s;
                    let b2 = r.y + 29 * s;
                    self.text.draw_style(fb, clip, tx, b1, r.right() - 20 * s, name, label_st, WHITE, 255);
                    self.text.draw_style(fb, clip, tx, b2, r.right() - 20 * s, &v, value_st, WHITE, pt::DIM_ALPHA);
                }
                None => {
                    let b = self.text.centre(r.y, r.height, value_st);
                    self.text.draw_style(fb, clip, tx, b, r.right() - 20 * s, name, value_st, WHITE, a);
                }
            }
            let csz = pt::CHEVRON * s;
            lucide::CHEVRON_RIGHT.draw(
                fb,
                clip,
                (r.right() - csz) as f32,
                (r.y + (r.height - csz) / 2) as f32,
                csz as f32,
                WHITE,
                a,
            );
        }
        // The volume slider.
        let sl = m.slider.scale(s);
        rounded_rect(fb, clip, sl, 6 * s, pt::SLIDER_FILL, pt::SLIDER_FILL_ALPHA);
        let (level, muted) = self.volume.unwrap_or((0, true));
        let icon = if muted { lucide::VOLUME_X } else { lucide::VOLUME_2 };
        let ia = if muted { 102 } else { 255 };
        let ic = m.icon.scale(s);
        icon.draw(fb, clip, ic.x as f32, ic.y as f32, ic.width as f32, WHITE, ia);
        let t = m.track.scale(s);
        rounded_rect(fb, clip, t, t.height / 2, WHITE, pt::SLIDER_REST_ALPHA);
        let filled = Rect::new(t.x, t.y, t.width * level as i32 / 100, t.height);
        if filled.width > 0 {
            let clip_fill = clip.intersect(&filled);
            rounded_rect(fb, clip_fill, t, t.height / 2, WHITE, 255);
        }
        // The settings button.
        let g = m.gear.scale(s);
        if self.panels.hover == Some(Hit::Gear) {
            rounded_rect(fb, clip, g, 6 * s, WHITE, ink::SELECTED);
        }
        let gsz = 15 * s;
        lucide::SETTINGS.draw(
            fb,
            clip,
            (g.x + (g.width - gsz) / 2) as f32,
            (g.y + (g.height - gsz) / 2) as f32,
            gsz as f32,
            WHITE,
            255,
        );
    }

    fn draw_sub_view(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32, v: ControlView, body: Rect) {
        let sv = self.sub_view(v, body);
        let b = sv.back.scale(s);
        let csz = 16 * s;
        lucide::CHEVRON_LEFT.draw(fb, clip, b.x as f32, (b.y + (b.height - csz) / 2) as f32, csz as f32, WHITE, 255);
        let hst = Style::new(Face::Medium, 14.0).scaled(s);
        let hb = self.text.centre(b.y, b.height, hst);
        self.text.draw_style(fb, clip, (b.x + csz + 8 * s) as f32, hb, body.right() * s, sv.title, hst, WHITE, 255);
        let gst = Style::new(Face::Sans, 11.0).scaled(s);
        let rst = Style::new(Face::Sans, 13.0).scaled(s);
        let mut i = 0;
        for (name, tb, rows) in &sv.groups {
            self.text.draw_style(
                fb,
                clip,
                ((body.x + 12 + 8) * s) as f32,
                tb * s,
                body.right() * s,
                name,
                gst,
                WHITE,
                ink::DIM,
            );
            for row in rows {
                let r = row.rect.scale(s);
                if self.panels.hover == Some(Hit::Row(i)) && !row.dim {
                    rounded_rect(fb, clip, r, 6 * s, WHITE, ink::SELECTED);
                }
                let a = if row.checked {
                    255
                } else if row.dim {
                    153
                } else {
                    179
                };
                let rb = self.text.centre(r.y, r.height, rst);
                self.text.draw_style(fb, clip, (r.x + 8 * s) as f32, rb, r.right() - 28 * s, &row.label, rst, WHITE, a);
                if row.checked {
                    let k = 14 * s;
                    lucide::CHECK.draw(
                        fb,
                        clip,
                        (r.right() - 8 * s - k) as f32,
                        (r.y + (r.height - k) / 2) as f32,
                        k as f32,
                        WHITE,
                        255,
                    );
                }
                i += 1;
            }
        }
    }

    /// The local date `day` days from today: (weekday 0 = Monday, day of
    /// the month, month 1-12).
    pub(super) fn calendar_date(&self, day: i32) -> (usize, u32, u32) {
        let (secs, off) = self.time;
        let c = crate::clock::civil(secs + i64::from(day) * 86_400, off);
        let days = (secs + i64::from(off) * 60).div_euclid(86_400) + i64::from(day);
        // 1970-01-01 was a Thursday.
        let weekday = (days + 3).rem_euclid(7) as usize;
        (weekday, c.day, c.month)
    }

    pub(super) fn calendar_layout(&self, body: Rect) -> CalendarLayout {
        let next = Rect::new(body.right() - 8 - 24, body.y + 10, 24, 24);
        let prev = next.translate(-26, 0);
        let today = (self.panels.day != 0).then(|| {
            let w = self.text.width("Today", 11.0) as i32 + 12;
            Rect::new(prev.x - 2 - w, prev.y, w, 24)
        });
        let day_row = Rect::new(body.x, body.y + pt::CAL_HEADER, body.width, pt::CAL_DAY);
        let all_day = Rect::new(body.x, day_row.bottom(), body.width, pt::CAL_ALL_DAY);
        let scroll = Rect::new(body.x, all_day.bottom(), body.width, pt::CAL_SCROLL);
        CalendarLayout { prev, next, today, day_row, all_day, scroll }
    }

    fn draw_calendar(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32, body: Rect) {
        let c = self.calendar_layout(body);
        let (weekday, day, month) = self.calendar_date(self.panels.day);
        let fs = s as f32;
        // The header: the date (14 px medium), the arrows.
        let hst = Style::new(Face::Medium, 14.0).scaled(s);
        let label = alloc::format!("{}, {} {}", WEEKDAYS[weekday], MONTHS[(month as usize + 11) % 12], day);
        let hb = self.text.centre((body.y + 10) * s, 24 * s, hst);
        self.text.draw_style(fb, clip, ((body.x + 12) * s) as f32, hb, c.prev.x * s, &label, hst, WHITE, 255);
        for (r, icon, enabled, hit) in [
            (c.prev, lucide::CHEVRON_LEFT, self.panels.day > -7, Hit::Prev),
            (c.next, lucide::CHEVRON_RIGHT, self.panels.day < 30, Hit::Next),
        ] {
            let r = r.scale(s);
            if self.panels.hover == Some(hit) && enabled {
                rounded_rect(fb, clip, r, 6 * s, WHITE, ink::SELECTED);
            }
            let k = 16 * s;
            icon.draw(
                fb,
                clip,
                (r.x + (r.width - k) / 2) as f32,
                (r.y + (r.height - k) / 2) as f32,
                k as f32,
                WHITE,
                if enabled { 255 } else { 77 },
            );
        }
        if let Some(t) = c.today {
            let t = t.scale(s);
            if self.panels.hover == Some(Hit::Today) {
                rounded_rect(fb, clip, t, 6 * s, WHITE, ink::SELECTED);
            }
            let st = Style::new(Face::Sans, 11.0).scaled(s);
            let b = self.text.centre(t.y, t.height, st);
            self.text.draw_style(fb, clip, (t.x + 6 * s) as f32, b, t.right(), "Today", st, WHITE, 255);
        }
        let col_x = (body.x + pt::CAL_GUTTER) * s;
        let border = |fb: &mut dyn Framebuffer, r: Rect| fill(fb, clip, r, WHITE, pt::CAL_BORDER_ALPHA);
        // The day: weekday (9 px capitals at 50 %), the date in a white
        // circle if it is today.
        let dr = c.day_row.scale(s);
        border(fb, Rect::new(dr.x, dr.bottom() - s, dr.width, s));
        border(fb, Rect::new(col_x, dr.y, s, dr.height));
        let col_w = pt::CAL_COLUMN * s;
        let wst = Style::new(Face::Sans, 9.0).scaled(s).tracking(0.025);
        let wd: String = WEEKDAYS[weekday][..3].to_uppercase();
        let ww = self.text.measure(&wd, wst);
        let wb = dr.y + 6 * s + (13.5 * fs / 2.0 + self.text.cap(wst) / 2.0) as i32;
        self.text.draw_style(
            fb,
            clip,
            col_x as f32 + (col_w as f32 - ww) / 2.0,
            wb,
            col_x + col_w,
            &wd,
            wst,
            WHITE,
            ink::DIM,
        );
        let cy = dr.y as f32 + (6.0 + 13.5 + 2.0 + 12.0) * fs;
        let cx = col_x as f32 + col_w as f32 / 2.0;
        let num = alloc::format!("{day}");
        if self.panels.day == 0 {
            poly_wayland::compose::paint::circle(fb, clip, cx, cy, 12.0 * fs, None, WHITE, 255);
            let nst = Style::new(Face::Semibold, 13.0).scaled(s);
            let nw = self.text.measure(&num, nst);
            let nb = (cy + self.text.cap(nst) / 2.0 + 0.5) as i32;
            self.text.draw_style(fb, clip, cx - nw / 2.0, nb, col_x + col_w, &num, nst, 0, 255);
        } else {
            let nst = Style::new(Face::Medium, 13.0).scaled(s);
            let nw = self.text.measure(&num, nst);
            let nb = (cy + self.text.cap(nst) / 2.0 + 0.5) as i32;
            self.text.draw_style(fb, clip, cx - nw / 2.0, nb, col_x + col_w, &num, nst, WHITE, 255);
        }
        // The all-day strip.
        let ad = c.all_day.scale(s);
        border(fb, Rect::new(ad.x, ad.bottom() - s, ad.width, s));
        border(fb, Rect::new(col_x, ad.y, s, ad.height));
        let sst = Style::new(Face::Sans, 9.0).scaled(s);
        let aw = self.text.measure("all-day", sst);
        let ab = ad.y + 4 * s + (13.5 * fs / 2.0 + self.text.cap(sst) / 2.0) as i32;
        self.text.draw_style(fb, clip, col_x as f32 - 6.0 * fs - aw, ab, col_x, "all-day", sst, WHITE, ink::DIM);
        // The hours, scrolled so the time now is in view.
        let sc = c.scroll.scale(s);
        let area = clip.intersect(&sc);
        border(fb, Rect::new(col_x, sc.y, s, sc.height).intersect(&area));
        let (secs, off) = self.time;
        let minutes = ((secs + i64::from(off) * 60).rem_euclid(86_400) / 60) as i32;
        let max_scroll = 24 * pt::CAL_HOUR - pt::CAL_SCROLL;
        let scroll = (minutes * pt::CAL_HOUR / 60 - 130).clamp(0, max_scroll);
        for hour in 0..=24 {
            let y = c.scroll.y + hour * pt::CAL_HOUR - scroll;
            if y < c.scroll.y - pt::CAL_HOUR || y > c.scroll.bottom() + pt::CAL_HOUR {
                continue;
            }
            let ys = y * s;
            fill(fb, area, Rect::new(col_x + s, ys, col_w, s), WHITE, pt::CAL_LINE_ALPHA);
            if (1..24).contains(&hour) {
                let label = alloc::format!("{hour:02}:00");
                let lw = self.text.measure(&label, sst);
                let lb = ys - 5 * s + (13.5 * fs / 2.0 + self.text.cap(sst) / 2.0) as i32;
                self.text.draw_style(fb, area, col_x as f32 - 6.0 * fs - lw, lb, col_x, &label, sst, WHITE, ink::DIM);
            }
        }
        if self.panels.day == 0 {
            let y = (c.scroll.y + minutes * pt::CAL_HOUR / 60 - scroll) * s;
            fill(fb, area, Rect::new(col_x, y, col_w, s), pt::CAL_NOW, 255);
            poly_wayland::compose::paint::circle(
                fb,
                area,
                col_x as f32,
                y as f32 + 0.5 * fs,
                3.0 * fs,
                None,
                pt::CAL_NOW,
                255,
            );
        }
    }

    pub(super) fn history_layout(&self, body: Rect) -> HistoryLayout {
        let n = self.notes.history_len().min(crate::theme::launcher::MAX_ROWS);
        let clear = (n > 0).then(|| {
            let w = self.text.width("Clear", 11.0) as i32 + 12;
            Rect::new(body.right() - 8 - w, body.y + 8, w, 24)
        });
        let first = body.y + 41 + 6;
        let rows = (0..n).map(|i| Rect::new(body.x + 6, first + i as i32 * 46, body.width - 12, 44)).collect();
        HistoryLayout { clear, rows }
    }

    fn draw_history(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32, body: Rect) {
        let h = self.history_layout(body);
        let hst = Style::new(Face::Medium, 14.0).scaled(s);
        let hb = self.text.centre(body.y * s, 40 * s, hst);
        self.text.draw_style(
            fb,
            clip,
            ((body.x + 12) * s) as f32,
            hb,
            body.right() * s,
            "Notifications",
            hst,
            WHITE,
            255,
        );
        if let Some(c) = h.clear {
            let c = c.scale(s);
            if self.panels.hover == Some(Hit::Clear) {
                rounded_rect(fb, clip, c, 6 * s, WHITE, ink::SELECTED);
            }
            let st = Style::new(Face::Sans, 11.0).scaled(s);
            let b = self.text.centre(c.y, c.height, st);
            self.text.draw_style(fb, clip, (c.x + 6 * s) as f32, b, c.right(), "Clear", st, WHITE, 200);
        }
        fill(
            fb,
            clip,
            Rect::new(body.x * s, (body.y + 40) * s, body.width * s, s),
            WHITE,
            crate::theme::launcher::DIVIDER_ALPHA,
        );
        let tst = Style::new(Face::Medium, 12.0).scaled(s);
        let mst = Style::new(Face::Sans, 12.0).scaled(s);
        let ast = Style::new(Face::Sans, 11.0).scaled(s);
        if h.rows.is_empty() {
            let r = Rect::new(body.x + 6, body.y + 47, body.width - 12, 32).scale(s);
            let st = Style::new(Face::Sans, 13.0).scaled(s);
            let b = self.text.centre(r.y, r.height, st);
            self.text.draw_style(fb, clip, (r.x + 8 * s) as f32, b, r.right(), "No notifications", st, WHITE, 153);
            return;
        }
        for (i, (row, n)) in h.rows.iter().zip(self.notes.history()).enumerate() {
            let r = row.scale(s);
            if self.panels.hover == Some(Hit::Row(i)) {
                rounded_rect(fb, clip, r, 6 * s, WHITE, ink::HOVER);
            }
            let time = self.age(n.arrived_ms);
            let tw = self.text.measure(&time, ast) as i32;
            let right = r.right() - 8 * s;
            let title = if n.summary.is_empty() { &n.app_name } else { &n.summary };
            let b1 = self.text.centre(r.y + 6 * s, 16 * s, tst);
            let b2 = self.text.centre(r.y + 22 * s, 16 * s, mst);
            self.text.draw_style(
                fb,
                clip,
                (r.x + 8 * s) as f32,
                b1,
                right - tw - 8 * s,
                title,
                tst,
                WHITE,
                nt::TITLE_ALPHA,
            );
            self.text.draw_style(fb, clip, (right - tw) as f32, b1, right, &time, ast, WHITE, ink::DIM);
            let line = n.body.lines().next().unwrap_or("");
            self.text.draw_style(fb, clip, (r.x + 8 * s) as f32, b2, right, line, mst, WHITE, nt::MESSAGE_ALPHA);
        }
    }

    /// How long ago something came, briefly.
    fn age(&self, at_ms: u64) -> String {
        let secs = self.now_ms.saturating_sub(at_ms) / 1000;
        match secs {
            0..60 => "now".into(),
            60..3600 => alloc::format!("{} min", secs / 60),
            _ => alloc::format!("{} h", secs / 3600),
        }
    }
}

/// A row of a list: its label, checked, dim.
type Choice = (String, bool, bool);

/// The control centre's main view.
pub(super) struct ControlMain {
    pub(super) items: Vec<(ControlView, Rect)>,
    pub(super) slider: Rect,
    pub(super) icon: Rect,
    pub(super) track: Rect,
    pub(super) gear: Rect,
    pub(super) height: i32,
}

pub(super) struct CalendarLayout {
    pub(super) prev: Rect,
    pub(super) next: Rect,
    pub(super) today: Option<Rect>,
    pub(super) day_row: Rect,
    pub(super) all_day: Rect,
    pub(super) scroll: Rect,
}

pub(super) struct HistoryLayout {
    pub(super) clear: Option<Rect>,
    pub(super) rows: Vec<Rect>,
}
