//! The shell: layout, drawing and input of the ridge, the launcher and the
//! wallpaper, as two compositor layers ([`Shell::below`] under the
//! windows, [`Shell::above`] over them).
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────────────────┐
//!  │ ◌                                          [ ▮▮▮ ◖) ] 21:41 Sep 12 │ ← the ridge, 24 px, rgb(32,32,32)
//!  ├──────────────────────────────────────────────────────────────────┤
//!  │                                                                  │
//!  │              ╭────────────────────────────────────╮              │
//!  │              │ ⌕  Search apps                      │ ← 48 px      │
//!  │              ├────────────────────────────────────┤              │
//!  │              │ Apps                                │              │
//!  │              │ ▢ Poly Demo                         │ ← rows 48 px │
//!  │              │   A Wayland window ...              │              │
//!  │              ├────────────────────────────────────┤              │
//!  │              │                    Open ↵  Close Esc│ ← footer 36  │
//!  │              ╰────────────────────────────────────╯              │
//!  │                      720 px wide, radius 14                      │
//!  └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Coordinates are logical; drawing multiplies them by the output's scale.
//! Everything that changes damages its rectangle ([`Shell::take_damage`]),
//! which the compositor hands to the scene, so only that is drawn again.

use crate::apps::App;
use crate::draw::{self, Text, fill, rounded_outline, rounded_rect};
use crate::launcher::{Key, Launcher, Outcome};
use crate::theme::{launcher as lt, ridge as rt, wallpaper as wt};
use alloc::string::String;
use alloc::vec::Vec;
use poly_wayland::compose::{Framebuffer, Layer, Rect};

const WHITE: u32 = 0xff_ffff;

/// What a pointer press did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Press {
    /// The shell took it.
    Taken,
    /// The shell took it: start this app.
    Launch(App),
    /// Not the shell's: the windows get it.
    NotMine,
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
}

impl Shell {
    /// A shell on a `width` × `height` logical output with these apps and
    /// fonts. The launcher shows at the start (an empty desktop).
    pub fn new(width: i32, height: i32, apps: Vec<App>, text: Text) -> Shell {
        let mut s = Shell {
            width,
            height,
            apps,
            launcher: Launcher::new(lt::MAX_ROWS),
            text,
            clock: String::new(),
            damage: Vec::new(),
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

    /// How many logical rows the ridge keeps free at the top.
    pub fn reserved_top(&self) -> i32 {
        rt::HEIGHT
    }

    pub fn ridge_rect(&self) -> Rect {
        Rect::new(0, 0, self.width, rt::HEIGHT)
    }

    /// The rectangles that changed since the last call, logical.
    pub fn take_damage(&mut self) -> Vec<Rect> {
        core::mem::take(&mut self.damage)
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
        let p = self.launcher_layout().panel;
        let s = lt::SHADOW_SPREAD;
        Rect::new(p.x - s, p.y - s, p.width + 2 * s, p.height + 2 * s + lt::SHADOW_OFFSET)
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

    /// The pointer is at logical (`x`, `y`): hovering a row selects it.
    pub fn pointer_motion(&mut self, x: i32, y: i32) {
        if !self.launcher.is_visible() {
            return;
        }
        let row = self.row_at(x, y);
        if self.launcher.hover() != row {
            self.change(|s| if s.launcher.set_hover(row) { Outcome::Changed } else { Outcome::Unchanged });
        }
    }

    /// A press of the left button at logical (`x`, `y`). While the launcher
    /// shows it takes every press: a row starts its app, outside the panel
    /// hides it (as clicking Alpine's transparent overlay does). The ridge
    /// takes presses too (it has no menus: the owner has not decided).
    pub fn press(&mut self, x: i32, y: i32) -> Press {
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
        if self.ridge_rect().contains(x, y) { Press::Taken } else { Press::NotMine }
    }

    /// The layer under the windows: the wallpaper and the ridge.
    pub fn below(&self) -> Below<'_> {
        Below(self)
    }

    /// The layer over the windows: the launcher (when shown).
    pub fn above(&self) -> Above<'_> {
        Above(self)
    }

    // -----------------------------------------------------------------
    // Drawing (physical pixels: logical × s).

    fn draw_wallpaper(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        let (w, h) = (self.width * s, self.height * s);
        let ch = |c: u32, sh: u32| ((c >> sh) & 0xff) as i32;
        // A 4×4 ordered dither (thresholds 0-15 of 16) hides the steps a
        // gradient this gentle would show in 8 bits.
        const BAYER: [i32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
        for y in clip.y..clip.bottom() {
            // Channels in 1/256 levels: the vertical gradient...
            let t = (y * 256 / h.max(1)).clamp(0, 256);
            let base = [16, 8, 0].map(|sh| ch(wt::TOP, sh) * (256 - t) + ch(wt::BOTTOM, sh) * t);
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
                    let v = base[i] + (ch(wt::GLOW, sh) * 256 - base[i]) * a / 1024;
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
        // The menu glyph on the left: a dashed-looking ring (Alpine's power
        // menu face), inert for now.
        let r = rt::ICON * s / 2 - s;
        draw::circle(fb, clip, (rt::MENU_ICON_X + rt::ICON / 2) * s, cy, r, Some(s.max(1)), WHITE, 200);
        // The clock pill on the right.
        let size = rt::TEXT_SIZE * s as f32;
        let tw = self.text.width(&self.clock, size) as i32;
        let pill_h = rt::PILL_HEIGHT * s;
        let clock_w = tw + 2 * rt::PILL_PADDING * s;
        let clock_x = (self.width - rt::RIGHT_MARGIN) * s - clock_w;
        let baseline = cy + (self.text.cap_height(size) as i32) / 2;
        self.text.draw(
            fb,
            clip,
            (clock_x + rt::PILL_PADDING * s) as f32,
            baseline,
            ridge.right(),
            &self.clock,
            size,
            rt::TEXT,
            255,
        );
        // The status pill left of it: network and volume placeholders.
        let icons = 2 * rt::ICON + rt::GAP;
        let status_w = (icons + 2 * rt::PILL_PADDING) * s;
        let status_x = clock_x - rt::GAP * s - status_w;
        let pill = Rect::new(status_x, cy - pill_h / 2, status_w, pill_h);
        rounded_rect(fb, clip, pill, pill_h / 2, WHITE, rt::PILL_ALPHA);
        let ix = status_x + rt::PILL_PADDING * s;
        // Network: three bars rising to the right.
        for i in 0..3 {
            let bh = (4 + 4 * i) * s;
            let bar = Rect::new(ix + (1 + 4 * i) * s, cy + 5 * s - bh, 3 * s, bh);
            fill(fb, clip, bar, WHITE, 230);
        }
        // Volume: a speaker and one wave.
        let vx = ix + (rt::ICON + rt::GAP) * s;
        fill(fb, clip, Rect::new(vx + s, cy - 2 * s, 3 * s, 4 * s), WHITE, 230);
        for i in 0..4 {
            fill(fb, clip, Rect::new(vx + (4 + i) * s, cy - (2 + i) * s, s, (4 + 2 * i) * s), WHITE, 230);
        }
        // The wave: the right half of a ring.
        let half = clip.intersect(&Rect::new(vx + 9 * s, cy - 7 * s, 6 * s, 14 * s));
        draw::circle(fb, half, vx + 7 * s, cy, 5 * s, Some(s), WHITE, 200);
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
        let radius = lt::RADIUS * s;
        // The shadow: layers growing outwards, each faint.
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
            rounded_rect(fb, clip, icon, 6 * s, tile_colour(&app.name), 255);
            let initial: String = app.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            let isize = 15.0 * s as f32;
            let iw = self.text.width(&initial, isize) as i32;
            let ibase = icon.y + ib / 2 + self.text.cap_height(isize) as i32 / 2;
            self.text.draw(fb, clip, (icon.x + (ib - iw) / 2) as f32, ibase, icon.right(), &initial, isize, WHITE, 255);
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
            let f = f.scale(s);
            fill(fb, clip, Rect::new(f.x, f.y - s, f.width, s), WHITE, lt::DIVIDER_ALPHA);
            let size = lt::FOOTER_SIZE * s as f32;
            let chip_size = 10.0 * s as f32;
            let base = f.y + f.height / 2 + self.text.cap_height(size) as i32 / 2;
            let mut x = f.right() - lt::SEARCH_PADDING * s;
            for (label, chip) in [("Close", "Esc"), ("Open", "\u{21b5}")] {
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
    }
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
        self.0.draw_launcher(fb, clip, scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::builtins;
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
}
