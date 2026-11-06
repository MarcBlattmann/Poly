//! The Alpine Shell look for Poly's own apps (ADR 0127): the shell's fonts,
//! its light or dark palette (`poly_shell::theme::pal`, chosen by the
//! settings' `theme`), and the pieces the apps build their windows from in
//! the settings window's layout language — the ground (surface 0) with a
//! 40-pixel header, a 224-pixel sidebar of 36-pixel rows, the page on
//! surface 1 with its rounded top-left corner and hairline, fields of
//! radius 8 on surface 2, ghost hovers, outline segmented controls,
//! popovers with Alpine's shadow — plus a one-line text editor for search
//! and rename fields.
//!
//! Layout is in logical pixels; everything here scales by the window's
//! integer scale when it draws, as the settings window does.

use alloc::string::String;
use alloc::vec::Vec;
use poly_shell::draw::{Face, Fonts, Style, Text, fill, rounded_border, rounded_rect};
use poly_shell::icons::Icon;
use poly_shell::theme::{pal, settings as t};
use poly_wayland::compose::paint::{Radii, Shadow};
use poly_wayland::compose::{Framebuffer, Rect};

/// Where the desktop's fonts are (the compositor's `SHELL_FONTS`).
pub const FONTS: &str = "/usr/share/poly/fonts";
/// The fallback font.
pub const FALLBACK_FONT: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

/// The header's height, the sidebar's width, the radii (the settings
/// window's).
pub const HEADER: i32 = t::HEADER;
pub const SIDEBAR: i32 = t::SIDEBAR;
pub const NAV_HEIGHT: i32 = t::NAV_HEIGHT;
pub const FIELD_RADIUS: i32 = t::FIELD_RADIUS;
pub const CARD_RADIUS: i32 = t::CARD_RADIUS;

/// Text styles: dense (13 px), small (12 px), labels (14 px), the page
/// title (22 px medium, `tracking-tight`), eyebrows (11 px semibold).
pub const DENSE: Style = Style::new(Face::Sans, t::DENSE_SIZE);
pub const DENSE_MEDIUM: Style = Style::new(Face::Medium, t::DENSE_SIZE);
pub const SMALL: Style = Style::new(Face::Sans, t::SMALL_SIZE);
pub const LABEL: Style = Style::new(Face::Sans, t::LABEL_SIZE);
pub const TITLE: Style = Style::new(Face::Medium, t::TITLE_SIZE).tracking(-0.025);
pub const EYEBROW: Style = Style::new(Face::Semibold, t::EYEBROW_SIZE).tracking(0.05);

/// The fonts and the scale an app draws with.
pub struct Ui {
    pub text: Text,
    pub scale: i32,
}

impl core::fmt::Debug for Ui {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ui").field("scale", &self.scale).finish_non_exhaustive()
    }
}

impl Ui {
    pub fn new(fonts: Fonts) -> Ui {
        Ui { text: Text::with_fonts(fonts), scale: 1 }
    }

    /// Logical `r` in physical pixels.
    pub fn px(&self, r: Rect) -> Rect {
        r.scale(self.scale)
    }

    /// The width of `s` in logical pixels.
    pub fn measure(&self, s: &str, st: Style) -> i32 {
        (self.text.measure(s, st.scaled(self.scale)) / self.scale as f32 + 0.999) as i32
    }

    /// `s` cut with an ellipsis to fit `max_w` logical pixels.
    pub fn fit(&self, s: &str, st: Style, max_w: i32) -> String {
        self.text.fit_style(s, st.scaled(self.scale), (max_w * self.scale) as f32)
    }

    /// Fill logical `r`.
    pub fn fill(&self, fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rgb: u32, alpha: u8) {
        fill(fb, clip, self.px(r), rgb, alpha);
    }

    /// A rounded rectangle, logical.
    #[allow(clippy::too_many_arguments)]
    pub fn round(&self, fb: &mut dyn Framebuffer, clip: Rect, r: Rect, radius: i32, rgb: u32, alpha: u8) {
        rounded_rect(fb, clip, self.px(r), radius * self.scale, rgb, alpha);
    }

    /// A one-logical-pixel border inside a rounded rectangle.
    #[allow(clippy::too_many_arguments)]
    pub fn border(&self, fb: &mut dyn Framebuffer, clip: Rect, r: Rect, radius: i32, rgb: u32, alpha: u8) {
        rounded_border(fb, clip, self.px(r), radius * self.scale, self.scale, rgb, alpha);
    }

    /// Text with its baseline centred in the logical row `top`..`top + h`,
    /// from `x`, cut at `max_x`; returns the logical pen position after it.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &self,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        x: i32,
        top: i32,
        h: i32,
        max_x: i32,
        s: &str,
        st: Style,
        rgb: u32,
    ) -> i32 {
        let s_ = self.scale;
        let st = st.scaled(s_);
        let base = self.text.centre(top * s_, h * s_, st);
        let end = self.text.draw_style(fb, clip, (x * s_) as f32, base, max_x * s_, s, st, rgb, 255);
        (end / s_ as f32) as i32
    }

    /// An icon of `size` logical pixels at (`x`, `y`).
    #[allow(clippy::too_many_arguments)]
    pub fn icon(
        &self,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        icon: Icon,
        x: i32,
        y: i32,
        size: i32,
        rgb: u32,
        alpha: u8,
    ) {
        let s = self.scale;
        icon.draw(fb, clip, (x * s) as f32, (y * s) as f32, (size * s) as f32, rgb, alpha);
    }

    /// The window's ground: surface 0, and the page on surface 1 right of
    /// the sidebar and below the header, with its rounded top-left corner
    /// and hairline (the settings window's).
    pub fn ground(&self, fb: &mut dyn Framebuffer, clip: Rect, w: i32, h: i32, page: Rect) {
        let s = self.scale;
        fill(fb, clip, Rect::new(0, 0, w * s, h * s), pal().s0, 255);
        let page = self.px(page);
        let r = (CARD_RADIUS * s) as f32;
        let grown = Rect::new(page.x, page.y, page.width + 2 * r as i32, page.height + 2 * r as i32);
        let clip = clip.intersect(&page);
        poly_wayland::compose::paint::fill_rounded(fb, clip, grown, Radii::all(r), pal().line, 255);
        let inner = Rect::new(grown.x + s, grown.y + s, grown.width - s, grown.height - s);
        poly_wayland::compose::paint::fill_rounded(fb, clip, inner, Radii::all(r - s as f32), pal().s1, 255);
    }

    /// A square button with an icon (the header's arrows): a ghost fill
    /// when hovered, faint when disabled.
    #[allow(clippy::too_many_arguments)]
    pub fn icon_button(
        &self,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        r: Rect,
        icon: Icon,
        size: i32,
        hover: bool,
        enabled: bool,
    ) {
        if hover && enabled {
            self.round(fb, clip, r, FIELD_RADIUS, pal().ink, pal().ghost);
        }
        let rgb = if enabled { pal().ink_primary } else { pal().faint };
        self.icon(fb, clip, icon, r.x + (r.width - size) / 2, r.y + (r.height - size) / 2, size, rgb, 255);
    }

    /// A field (search, path, rename): surface 2, radius 8, a hairline
    /// that is stronger while it has the keyboard.
    pub fn field(&self, fb: &mut dyn Framebuffer, clip: Rect, r: Rect, focused: bool) {
        self.round(fb, clip, r, FIELD_RADIUS, pal().s2, 255);
        let line = if focused { pal().faint } else { pal().line };
        self.border(fb, clip, r, FIELD_RADIUS, line, 255);
    }

    /// A sidebar row: an icon 15 pixels and the label 13 pixels, the
    /// active one medium on surface 3, a hovered one on a ghost fill.
    #[allow(clippy::too_many_arguments)]
    pub fn nav_row(
        &self,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        r: Rect,
        icon: Icon,
        label: &str,
        active: bool,
        hover: bool,
    ) {
        if active {
            self.round(fb, clip, r, FIELD_RADIUS, pal().s3, 255);
        } else if hover {
            self.round(fb, clip, r, FIELD_RADIUS, pal().ink, pal().ghost);
        }
        let ink = if active { pal().ink_primary } else { pal().quiet };
        let x = r.x + t::NAV_PADDING;
        self.icon(fb, clip, icon, x, r.y + (r.height - t::NAV_ICON) / 2, t::NAV_ICON, ink, 255);
        let st = if active { DENSE_MEDIUM } else { DENSE };
        let label = self.fit(label, st, r.right() - x - t::NAV_ICON - 10 - t::NAV_PADDING);
        self.text(fb, clip, x + t::NAV_ICON + 10, r.y, r.height, r.right(), &label, st, ink);
    }

    /// An eyebrow (a section label in the sidebar): 11 px semibold, faint.
    pub fn eyebrow(&self, fb: &mut dyn Framebuffer, clip: Rect, x: i32, y: i32, label: &str) {
        self.text(fb, clip, x, y, 18, x + 200, label, EYEBROW, pal().faint);
    }

    /// An outline segmented control (`rounded-full`, 13 px): `options` are
    /// (icon, active); returns their rectangles.
    pub fn segmented(&self, fb: &mut dyn Framebuffer, clip: Rect, r: Rect, options: &[(Icon, bool)]) -> Vec<Rect> {
        self.border(fb, clip, r, r.height / 2, pal().line, 255);
        let n = options.len().max(1) as i32;
        let w = (r.width - 4) / n;
        let mut out = Vec::new();
        for (i, (icon, active)) in options.iter().enumerate() {
            let o = Rect::new(r.x + 2 + i as i32 * w, r.y + 2, w, r.height - 4);
            if *active {
                self.round(fb, clip, o, o.height / 2, pal().ink_primary, 255);
            }
            let ink = if *active { pal().on_solid } else { pal().quiet };
            let size = 14;
            self.icon(fb, clip, *icon, o.x + (o.width - size) / 2, o.y + (o.height - size) / 2, size, ink, 255);
            out.push(o);
        }
        out
    }

    /// A popover (context menus): surface 2 with a hairline, radius 12,
    /// Alpine's popover shadow.
    pub fn popover(&self, fb: &mut dyn Framebuffer, clip: Rect, r: Rect) {
        let s = self.scale;
        let p = self.px(r);
        Shadow::new(0.0, 8.0, 24.0, 0.0, 0, 90).scaled(s).draw(
            fb,
            clip,
            p,
            Radii::all((10 * s) as f32),
            Rect::default(),
        );
        self.round(fb, clip, r, 10, pal().s2, 255);
        self.border(fb, clip, r, 10, pal().line, 255);
    }
}

/// The desktop's settings an app follows: the theme and the keyboard
/// layout, from the shell's settings file (`$POLY_SHELL_SETTINGS`, which
/// the compositor gives the apps it starts, else
/// `$XDG_CONFIG_HOME/poly/shell.conf`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub light: bool,
    pub layout: String,
}

impl Look {
    /// From the settings file's text.
    pub fn parse(text: &str) -> Look {
        let s = poly_shell::settings::Settings::parse(text);
        Look { light: s.theme == "light", layout: s.keyboard }
    }

    pub fn layout(&self) -> &'static poly_input::layout::Layout {
        poly_input::layout::by_name(&self.layout).unwrap_or(&poly_input::layout::US)
    }
}

impl Default for Look {
    fn default() -> Look {
        Look::parse("")
    }
}

/// One line of editable text with a cursor (search, rename, new folder).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEdit {
    pub text: String,
    /// The cursor, a byte offset on a character boundary.
    pub cursor: usize,
    /// Everything is selected (typing replaces it), as a field shows its
    /// text when it opens.
    pub all: bool,
}

/// What a key did to a [`LineEdit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// The text changed.
    Changed,
    /// Only the cursor moved.
    Moved,
    Enter,
    Escape,
    /// Not a key for the field.
    Ignored,
}

impl LineEdit {
    pub fn new(text: &str) -> LineEdit {
        LineEdit { text: text.into(), cursor: text.len(), all: false }
    }

    /// `text` with everything selected, the cursor before `stem_end` (the
    /// name without its extension, as a rename field opens).
    pub fn selecting(text: &str, stem_end: usize) -> LineEdit {
        LineEdit { text: text.into(), cursor: stem_end.min(text.len()), all: true }
    }

    fn prev(&self) -> usize {
        self.text[..self.cursor].char_indices().next_back().map_or(0, |(i, _)| i)
    }

    fn next(&self) -> usize {
        self.text[self.cursor..].chars().next().map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }

    /// The start of the word before the cursor.
    fn word_start(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end_matches(|c: char| !c.is_alphanumeric());
        trimmed.rfind(|c: char| !c.is_alphanumeric()).map_or(0, |i| i + 1)
    }

    /// Insert `s` at the cursor (replacing everything if it is selected).
    pub fn insert(&mut self, s: &str) {
        if self.all {
            self.text.clear();
            self.cursor = 0;
            self.all = false;
        }
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// A key: characters type, Backspace and Delete erase (Ctrl: a word),
    /// arrows, Home and End move, Ctrl+A selects all, Ctrl+U clears.
    pub fn key(&mut self, k: &crate::input::KeyInput) -> Edit {
        use poly_input::key::Key;
        if !k.pressed {
            return Edit::Ignored;
        }
        let ctrl = k.mods.ctrl;
        match k.key {
            Some(Key::Enter | Key::NumpadEnter) => return Edit::Enter,
            Some(Key::Escape) => return Edit::Escape,
            Some(Key::Backspace) => {
                if self.all {
                    self.text.clear();
                    self.cursor = 0;
                    self.all = false;
                    return Edit::Changed;
                }
                let from = if ctrl { self.word_start() } else { self.prev() };
                if from == self.cursor {
                    return Edit::Moved;
                }
                self.text.replace_range(from..self.cursor, "");
                self.cursor = from;
                return Edit::Changed;
            }
            Some(Key::Delete) => {
                if self.all {
                    self.text.clear();
                    self.cursor = 0;
                    self.all = false;
                    return Edit::Changed;
                }
                let to = self.next();
                if to == self.cursor {
                    return Edit::Moved;
                }
                self.text.replace_range(self.cursor..to, "");
                return Edit::Changed;
            }
            Some(Key::ArrowLeft) => {
                self.cursor = if self.all { 0 } else { self.prev() };
                self.all = false;
                return Edit::Moved;
            }
            Some(Key::ArrowRight) => {
                self.cursor = if self.all { self.text.len() } else { self.next() };
                self.all = false;
                return Edit::Moved;
            }
            Some(Key::Home) => {
                self.cursor = 0;
                self.all = false;
                return Edit::Moved;
            }
            Some(Key::End) => {
                self.cursor = self.text.len();
                self.all = false;
                return Edit::Moved;
            }
            _ => {}
        }
        if ctrl {
            return match k.ch {
                Some('a' | 'A') => {
                    self.all = true;
                    Edit::Moved
                }
                Some('u' | 'U') => {
                    self.text.clear();
                    self.cursor = 0;
                    self.all = false;
                    Edit::Changed
                }
                _ => Edit::Ignored,
            };
        }
        if k.mods.alt || k.mods.meta {
            return Edit::Ignored;
        }
        match k.ch {
            Some(c) => {
                let mut b = [0u8; 4];
                self.insert(c.encode_utf8(&mut b));
                Edit::Changed
            }
            None => Edit::Ignored,
        }
    }

    /// Draw the text in a field at logical `r` (with `placeholder` when
    /// empty), the cursor when `focused`, the selection when all is.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        ui: &Ui,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        r: Rect,
        st: Style,
        placeholder: &str,
        focused: bool,
    ) {
        let x = r.x;
        if self.text.is_empty() {
            ui.text(fb, clip, x, r.y, r.height, r.right(), placeholder, st, pal().faint);
        } else {
            if self.all && focused {
                let w = ui.measure(&self.text, st).min(r.width);
                ui.round(fb, clip, Rect::new(x - 2, r.y + 4, w + 4, r.height - 8), 4, pal().ink, 60);
            }
            ui.text(fb, clip, x, r.y, r.height, r.right(), &self.text, st, pal().ink_primary);
        }
        if focused && !self.all {
            let cx = x + ui.measure(&self.text[..self.cursor], st);
            ui.fill(fb, clip, Rect::new(cx.min(r.right() - 1), r.y + 6, 1, r.height - 12), pal().ink_primary, 255);
        }
    }
}

/// A context menu: rows of a label and an optional key hint; separators.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Menu {
    pub items: Vec<MenuItem>,
    /// Top left, logical.
    pub at: (i32, i32),
    pub hover: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub hint: String,
    pub enabled: bool,
    /// A separator line (label ignored).
    pub separator: bool,
    /// What the app does for it.
    pub id: u32,
}

impl MenuItem {
    pub fn new(id: u32, label: &str, hint: &str, enabled: bool) -> MenuItem {
        MenuItem { label: label.into(), hint: hint.into(), enabled, separator: false, id }
    }

    pub fn separator() -> MenuItem {
        MenuItem { label: String::new(), hint: String::new(), enabled: false, separator: true, id: 0 }
    }
}

/// A menu row's height, the separators', the menu's width and padding.
pub const MENU_ROW: i32 = 28;
pub const MENU_SEPARATOR: i32 = 9;
pub const MENU_WIDTH: i32 = 220;
pub const MENU_PAD: i32 = 5;

impl Menu {
    /// A menu at (`x`, `y`), moved to stay inside a `w` × `h` window.
    pub fn new(items: Vec<MenuItem>, x: i32, y: i32, w: i32, h: i32) -> Menu {
        let mut m = Menu { items, at: (x, y), hover: None };
        let r = m.rect();
        m.at.0 = x.min(w - r.width - 4).max(4);
        m.at.1 = if y + r.height > h - 4 { (y - r.height).max(4) } else { y };
        m
    }

    pub fn rect(&self) -> Rect {
        let h: i32 = self.items.iter().map(|i| if i.separator { MENU_SEPARATOR } else { MENU_ROW }).sum();
        Rect::new(self.at.0, self.at.1, MENU_WIDTH, h + 2 * MENU_PAD)
    }

    /// The rows' rectangles (separators included).
    fn rows(&self) -> Vec<Rect> {
        let mut y = self.at.1 + MENU_PAD;
        self.items
            .iter()
            .map(|i| {
                let h = if i.separator { MENU_SEPARATOR } else { MENU_ROW };
                let r = Rect::new(self.at.0 + MENU_PAD, y, MENU_WIDTH - 2 * MENU_PAD, h);
                y += h;
                r
            })
            .collect()
    }

    /// The enabled item at (`x`, `y`).
    pub fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        self.rows().iter().position(|r| r.contains(x, y)).filter(|&i| self.items[i].enabled && !self.items[i].separator)
    }

    pub fn draw(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect) {
        ui.popover(fb, clip, self.rect());
        for (i, (item, r)) in self.items.iter().zip(self.rows()).enumerate() {
            if item.separator {
                ui.fill(fb, clip, Rect::new(r.x + 4, r.y + 4, r.width - 8, 1), pal().ink, pal().hairline);
                continue;
            }
            if self.hover == Some(i) && item.enabled {
                ui.round(fb, clip, r, 6, pal().ink, pal().ghost);
            }
            let ink = if item.enabled { pal().ink_primary } else { pal().faint };
            ui.text(fb, clip, r.x + 10, r.y, r.height, r.right() - 60, &item.label, DENSE, ink);
            if !item.hint.is_empty() {
                let mono = Style::new(Face::Mono, 11.0);
                let w = ui.measure(&item.hint, mono);
                ui.text(fb, clip, r.right() - 10 - w, r.y, r.height, r.right(), &item.hint, mono, pal().faint);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{KeyInput, Mods};
    use poly_input::key::Key;

    fn key(k: Option<Key>, ch: Option<char>, ctrl: bool) -> KeyInput {
        KeyInput { code: 0, key: k, ch, mods: Mods { ctrl, ..Mods::default() }, pressed: true, repeat: false }
    }

    #[test]
    fn line_edit_types_moves_and_erases() {
        let mut e = LineEdit::new("ab");
        assert_eq!(e.key(&key(None, Some('c'), false)), Edit::Changed);
        assert_eq!(e.text, "abc");
        e.key(&key(Some(Key::ArrowLeft), None, false));
        e.key(&key(None, Some('é'), false));
        assert_eq!((e.text.as_str(), e.cursor), ("abéc", 4));
        e.key(&key(Some(Key::Backspace), None, false));
        assert_eq!(e.text, "abc");
        e.key(&key(Some(Key::Home), None, false));
        e.key(&key(Some(Key::Delete), None, false));
        assert_eq!((e.text.as_str(), e.cursor), ("bc", 0));
        assert_eq!(e.key(&key(Some(Key::Enter), None, false)), Edit::Enter);
        assert_eq!(e.key(&key(Some(Key::Escape), None, false)), Edit::Escape);
        assert_eq!(e.key(&key(None, Some('x'), true)), Edit::Ignored, "Ctrl+X is not typed");
    }

    #[test]
    fn line_edit_words_and_selection() {
        let mut e = LineEdit::new("hello big world");
        e.key(&key(Some(Key::Backspace), None, true));
        assert_eq!(e.text, "hello big ");
        e.key(&key(Some(Key::Backspace), None, true));
        assert_eq!(e.text, "hello ");
        let mut r = LineEdit::selecting("report.txt", 6);
        assert!(r.all);
        r.key(&key(None, Some('n'), false));
        assert_eq!(r.text, "n", "typing replaces a selected name");
        let mut r = LineEdit::selecting("report.txt", 6);
        r.key(&key(Some(Key::ArrowLeft), None, false));
        assert_eq!((r.cursor, r.all), (0, false));
        r.key(&key(None, Some('a'), true));
        assert!(r.all);
        r.key(&key(None, Some('u'), true));
        assert_eq!(r.text, "");
    }

    #[test]
    fn menus_stay_in_the_window_and_skip_disabled_rows() {
        let items = alloc::vec![
            MenuItem::new(1, "Open", "Enter", true),
            MenuItem::separator(),
            MenuItem::new(2, "Paste", "Ctrl+V", false),
            MenuItem::new(3, "Rename", "F2", true),
        ];
        let m = Menu::new(items, 790, 590, 800, 600);
        let r = m.rect();
        assert!(r.right() <= 796 && r.bottom() <= 596, "{r:?}");
        let first = m.at.1 + MENU_PAD + 5;
        assert_eq!(m.item_at(m.at.0 + 20, first), Some(0));
        let paste = m.at.1 + MENU_PAD + MENU_ROW + MENU_SEPARATOR + 5;
        assert_eq!(m.item_at(m.at.0 + 20, paste), None, "disabled");
        assert_eq!(m.item_at(m.at.0 + 20, paste + MENU_ROW), Some(3));
        assert_eq!(m.item_at(m.at.0 + 20, m.at.1 + MENU_PAD + MENU_ROW + 2), None, "separator");
    }

    #[test]
    fn look_follows_the_settings_file() {
        let l = Look::parse("theme=light\nkeyboard=de-ch\n");
        assert!(l.light);
        assert_eq!(l.layout().name, poly_input::layout::DE_CH.name);
        assert!(!Look::default().light);
    }
}
