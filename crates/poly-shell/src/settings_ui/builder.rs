//! Page building: the controls, rows and cards a page is made of.

use super::*;

/// A row of a card: label, description, and what goes on its right.
pub(super) enum Control {
    Value(String),
    Segmented(Vec<(String, bool, Target)>),
    Slider(u32, Option<Target>),
    Toggle(bool, Option<Target>),
    /// A shortcut: its caps (or "Press a shortcut"), its reset, Set.
    Binding {
        action: Action,
        refused: bool,
    },
}

/// Builds a page's items. `y` is the top of the next thing.
pub(super) struct Builder<'a, 'b> {
    pub(super) cx: &'a Context<'b>,
    pub(super) items: Vec<Item>,
    pub(super) targets: Vec<(Rect, Target)>,
    pub(super) hover: Option<Target>,
    /// The shortcut being recorded, and the last refusal.
    pub(super) recording: Option<Action>,
    pub(super) refusal: Option<(Action, String)>,
    pub(super) left: i32,
    pub(super) right: i32,
    pub(super) y: i32,
}

const LABEL: Style = Style::new(Face::Sans, t::LABEL_SIZE);
pub(super) const DESC: Style = Style::new(Face::Sans, t::SMALL_SIZE);
pub(super) const DENSE: Style = Style::new(Face::Sans, t::DENSE_SIZE);

impl Builder<'_, '_> {
    pub(super) fn text(&mut self, x: i32, y: i32, max_x: i32, text: &str, style: Style, rgb: u32) {
        self.items.push(Item::Text { x, y, max_x, text: text.into(), style, rgb });
    }

    /// The baseline that centres a line of `st` in `top..top + h`.
    pub(super) fn baseline(&self, top: i32, h: i32, st: Style) -> i32 {
        self.cx.text.centre(top, h, st)
    }

    /// The page's title: Geist 22 px medium, `tracking-tight`, a 32-pixel
    /// line; the first section 32 pixels below it.
    pub(super) fn title(&mut self, title: &str) {
        let st = Style::new(Face::Medium, t::TITLE_SIZE).tracking(-0.025);
        let b = self.baseline(self.y, 32, st);
        self.text(self.left, b, self.right, title, st, pal().ink_primary);
        self.y += 32 + 32;
    }

    /// A section's label (11 px medium, quiet, `pl-1`), 8 pixels above
    /// its card.
    pub(super) fn label(&mut self, label: &str) {
        let st = Style::new(Face::Medium, t::EYEBROW_SIZE);
        let b = self.baseline(self.y, 16, st);
        self.text(self.left + 4, b, self.right, label, st, pal().quiet);
        self.y += 16 + 8;
    }

    /// A card of rows (Alpine's group card: `px-3.5 py-2.5`, at least 48
    /// pixels, hairlines between): (label, description, control).
    pub(super) fn rows(&mut self, rows: Vec<(String, Option<String>, Control)>) {
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
                    self.text(x, b, max, &label, LABEL, pal().ink_primary);
                }
                Some(d) => {
                    let b = self.baseline(r.y + 10, 20, LABEL);
                    self.text(x, b, max, &label, LABEL, pal().ink_primary);
                    let b = self.baseline(r.y + 32, 16, DESC);
                    let rgb = if matches!(control, Control::Binding { refused: true, .. }) {
                        pal().danger
                    } else {
                        pal().quiet
                    };
                    self.text(x, b, max, d, DESC, rgb);
                }
            }
            self.control(r, control);
            y += h;
        }
        self.y = card.bottom() + 32;
    }

    pub(super) fn control(&mut self, r: Rect, c: Control) {
        let right = r.right() - t::ROW_PADDING;
        let mid = r.y + r.height / 2;
        match c {
            Control::Value(v) => {
                let room = (right - (r.x + 200)) as f32;
                let v = self.cx.text.fit_style(&v, DENSE, room);
                let w = self.cx.text.measure(&v, DENSE) as i32;
                let b = self.baseline(r.y, r.height, DENSE);
                self.text(right - w, b, right + 1, &v, DENSE, pal().quiet);
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
                self.text(right - w, b, right + 1, &v, DESC, pal().quiet);
                if let Some(tg) = target {
                    self.targets.push((Rect::new(track.x - 6, r.y, track.width + 12, r.height), tg));
                }
            }
            Control::Binding { action, .. } => {
                // Alpine's `BindingRow`: the caps (or "Press a shortcut"
                // while recording), the reset when it differs from the
                // default, then Set or Cancel; 8 pixels apart.
                let recording = self.recording == Some(action);
                let label = if recording { "Cancel" } else { "Set" };
                let bst = Style::new(Face::Medium, t::DENSE_SIZE);
                let bw = self.cx.text.measure(label, bst) as i32 + 24;
                let btn = Rect::new(right - bw, mid - 14, bw, 28);
                let hover = self.hover == Some(Target::Record(action));
                self.items.push(Item::Button { rect: btn, label: label.into(), hover });
                self.targets.push((btn, Target::Record(action)));
                let mut x = btn.x - 8;
                if !recording && self.cx.settings.shortcuts.is_custom(action) {
                    let ib = Rect::new(x - 28, mid - 14, 28, 28);
                    let hover = self.hover == Some(Target::ResetShortcut(action));
                    self.items.push(Item::IconButton {
                        rect: ib,
                        icon: lucide::ROTATE_CCW,
                        size: 14,
                        hover,
                        enabled: true,
                    });
                    self.targets.push((ib, Target::ResetShortcut(action)));
                    x = ib.x - 8;
                }
                if recording {
                    let st = Style::new(Face::Sans, 11.0);
                    let text = "Press a shortcut";
                    let w = self.cx.text.measure(text, st) as i32 + 16;
                    self.items.push(Item::Pill { rect: Rect::new(x - w, mid - 11, w, 22), text: text.into() });
                } else {
                    let keys = self.cx.settings.shortcuts.chord(action).caps();
                    self.caps(x, mid, &keys);
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

    /// Key caps ending at `right`, centred on `mid`, 4 pixels apart.
    pub(super) fn caps(&mut self, right: i32, mid: i32, keys: &[String]) {
        let mono = Style::new(Face::Mono, 11.0);
        let mut caps = Vec::new();
        let mut x = right;
        for k in keys.iter().rev() {
            let w = (self.cx.text.measure(k, mono) as i32 + 12).max(22);
            x -= w;
            caps.push((Rect::new(x, mid - 11, w, 22), k.clone()));
            x -= 4;
        }
        caps.reverse();
        self.items.push(Item::Caps(caps));
    }

    /// A card of shortcuts (Alpine's `ShortcutsPage` rows): each with its
    /// description, or the refusal of its last recording in red.
    pub(super) fn bindings(&mut self, rows: &[(Action, Option<&str>)]) {
        let rows = rows
            .iter()
            .map(|&(action, desc)| {
                let refusal = self.refusal.as_ref().filter(|(a, _)| *a == action).map(|(_, m)| m.clone());
                let refused = refusal.is_some();
                (action.label().to_string(), refusal.or(desc.map(String::from)), Control::Binding { action, refused })
            })
            .collect();
        self.rows(rows);
    }

    /// A card of choices (48-pixel rows, 36 for long lists): label,
    /// detail on the right, a check if chosen.
    pub(super) fn choices(&mut self, rows: &[(String, String, bool, Target)]) {
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
                self.items.push(Item::Fill { rect: r, rgb: pal().ink, alpha: pal().ghost, radius });
            }
            let b = self.baseline(r.y, r.height, DENSE);
            self.text(r.x + t::ROW_PADDING, b, r.right() - 140, label, DENSE, pal().ink_primary);
            let check_x = r.right() - t::ROW_PADDING - 16;
            if *chosen {
                self.items.push(Item::Icon {
                    icon: lucide::CHECK,
                    x: check_x,
                    y: r.y + (rh - 16) / 2,
                    size: 16,
                    rgb: pal().ink_primary,
                    alpha: 255,
                });
            }
            if !detail.is_empty() {
                let w = self.cx.text.measure(detail, DESC) as i32;
                let b = self.baseline(r.y, r.height, DESC);
                self.text(check_x - 12 - w, b, check_x - 4, detail, DESC, pal().quiet);
            }
            self.targets.push((r, *target));
        }
        self.y = card.bottom() + 32;
    }

    /// A card of rows that lead to pages: icon, label, detail, chevron.
    pub(super) fn links(&mut self, rows: &[(Page, String)]) {
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
                self.items.push(Item::Fill { rect: r, rgb: pal().ink, alpha: pal().ghost, radius });
            }
            let ix = r.x + t::ROW_PADDING;
            self.items.push(Item::Icon {
                icon: page.icon(),
                x: ix,
                y: r.y + 16,
                size: 15,
                rgb: pal().quiet,
                alpha: 255,
            });
            let b = self.baseline(r.y, r.height, DENSE);
            self.text(ix + 15 + 12, b, r.right() - 160, page.label(), DENSE, pal().ink_primary);
            let cx = r.right() - t::ROW_PADDING - 16;
            self.items.push(Item::Icon {
                icon: lucide::CHEVRON_RIGHT,
                x: cx,
                y: r.y + 16,
                size: 16,
                rgb: pal().faint,
                alpha: 255,
            });
            if !detail.is_empty() {
                let w = self.cx.text.measure(detail, DENSE) as i32;
                self.text(cx - 12 - w, b, cx - 4, detail, DENSE, pal().quiet);
            }
            self.targets.push((r, Target::Nav(*page)));
        }
        self.y = card.bottom() + 32;
    }

    /// A note (12 px, faint).
    pub(super) fn note(&mut self, text: &str) {
        let b = self.baseline(self.y - 24, 16, DESC);
        self.text(self.left + 4, b, self.right, text, DESC, pal().faint);
        self.y += 8;
    }
}
