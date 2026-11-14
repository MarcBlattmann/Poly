//! The Terminal window (ADR 0134): Alpine's look — a 40-pixel header on
//! the ground (surface 0) with the terminal's icon and title and the
//! Copy, Paste and New window buttons, the hairline, then the screen on the
//! page (surface 1) with 12 pixels around it — and the emulation of
//! `poly-vt` drawn in Geist Mono, 13 pixels, in cells as wide as its
//! advance and 1.35 em high.
//!
//! - **Keys** go to the shell as xterm sends them (`poly_vt::keys`);
//!   Ctrl+Shift+C and Ctrl+Shift+V copy and paste, Shift+Insert pastes,
//!   Shift+Page Up/Down scroll the history, any other key snaps back to
//!   the live screen.
//! - **The mouse** selects (a drag; a double click a word, a triple click a
//!   line), copies with Ctrl+Shift+C or the menu (right button), pastes
//!   with the middle button; with Shift, or when the program asks for the
//!   mouse (`CSI ? 1000 h` and friends), it goes to the program.
//! - **The wheel** scrolls the history three lines a notch; on the
//!   alternate screen (`less`, `vim`) it sends arrow keys, as xterm's
//!   `alternateScroll`.
//! - **Colours**: the default foreground and background are the theme's
//!   ink and page; the 16 ANSI colours are Tailwind's 400 tones on the dark
//!   theme and 600 on the light (Alpine's palette is Tailwind's), 16–255
//!   xterm's cube and greys, direct colours as given.

use crate::pty::Pty;
use alloc::string::String;
use alloc::vec::Vec;
use poly_app::{App, AppEvent, CursorShape, Cx, KeyInput, Menu, MenuItem, Ui};
use poly_input::key::Key;
use poly_shell::draw::{Face, Style};
use poly_shell::icons::lucide;
use poly_shell::theme::pal;
use poly_vt::keys::{self, KeyMods, MouseAction, MouseButton, MouseEvent};
use poly_vt::{Attrs, Cell, Color, CursorShape as VtCursor, Damage, MouseMode, Terminal};
use poly_wayland::compose::{Framebuffer, Rect, Scale};

/// The header's height and the padding around the screen, logical.
pub const HEADER: i32 = 40;
pub const PAD: i32 = 12;
/// The text size, logical pixels per em, and the line height in em.
pub const FONT_SIZE: f32 = 13.0;
pub const LINE: f32 = 1.35;
/// Lines of history kept.
pub const SCROLLBACK: usize = 5000;

/// The mono style at scale `s`.
fn mono(s: Scale) -> Style {
    Style::new(Face::Mono, FONT_SIZE).scaled(s)
}

/// The ANSI colours 0–15 of a theme.
pub fn ansi(light: bool) -> [u32; 16] {
    if light {
        [
            0x2f_2f2f, 0xdc_2626, 0x16_a34a, 0xca_8a04, 0x25_63eb, 0xc0_26d3, 0x08_91b2, 0xb4_b4b4, 0x6f_6f6f,
            0xef_4444, 0x22_c55e, 0xea_b308, 0x3b_82f6, 0xd9_46ef, 0x06_b6d4, 0x91_9191,
        ]
    } else {
        [
            0x33_3332, 0xf8_7171, 0x4a_de80, 0xfa_cc15, 0x60_a5fa, 0xe8_79f9, 0x22_d3ee, 0xd4_d4d0, 0x6f_6f6b,
            0xfc_a5a5, 0x86_efac, 0xfd_e047, 0x93_c5fd, 0xf0_abfc, 0x67_e8f9, 0xff_ffff,
        ]
    }
}

/// A colour as RGB, `fg` choosing the default for text or ground.
pub fn rgb(c: Color, fg: bool, light: bool) -> u32 {
    match c {
        Color::Default => {
            if fg {
                pal().ink_primary
            } else {
                pal().s1
            }
        }
        Color::Indexed(i) if i < 16 => ansi(light)[usize::from(i)],
        Color::Indexed(i) => {
            let (r, g, b) = poly_vt::palette::xterm_256(i);
            u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
        }
        Color::Rgb(r, g, b) => u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
    }
}

/// A cell's ink and ground, after inverse, bold-as-bright and faint.
pub fn cell_colors(c: &Cell, light: bool, reverse_video: bool) -> (u32, u32) {
    let fg_c = match c.fg {
        // Bold with one of the eight colours shows its bright one.
        Color::Indexed(i) if i < 8 && c.attrs.contains(Attrs::BOLD) => Color::Indexed(i + 8),
        other => other,
    };
    let (mut fg, mut bg) = (rgb(fg_c, true, light), rgb(c.bg, false, light));
    if c.attrs.contains(Attrs::INVERSE) != reverse_video {
        core::mem::swap(&mut fg, &mut bg);
    }
    if c.attrs.contains(Attrs::FAINT) {
        fg = poly_shell::draw::mix(bg | 0xff00_0000, fg, 150) & 0xff_ffff;
    }
    if c.attrs.contains(Attrs::HIDDEN) {
        fg = bg;
    }
    (fg, bg)
}

/// The cell grid on a window: cell size in physical pixels, the screen's
/// origin, its columns and rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Grid {
    pub cell_w: i32,
    pub cell_h: i32,
    /// Top left of the first cell, physical.
    pub x: i32,
    pub y: i32,
    pub cols: u16,
    pub rows: u16,
}

impl Grid {
    /// The grid for a `w` × `h` logical window at scale `s`, cells of
    /// `advance` physical pixels (the mono face's).
    pub fn new(w: i32, h: i32, s: Scale, advance: f32) -> Grid {
        let cell_w = ((advance + 0.5) as i32).max(1);
        let cell_h = ((FONT_SIZE * LINE * s.factor() + 0.5) as i32).max(1);
        let (x, y) = (PAD * s, (HEADER + PAD) * s);
        let cols = ((s.px(w) - 2 * PAD * s) / cell_w).clamp(2, 1000) as u16;
        let rows = ((s.px(h - HEADER) - 2 * PAD * s) / cell_h).clamp(2, 500) as u16;
        Grid { cell_w, cell_h, x, y, cols, rows }
    }

    /// Row `r`'s rectangle, physical, across the whole page.
    pub fn row_rect(&self, r: u16, page_w: i32) -> Rect {
        Rect::new(0, self.y + i32::from(r) * self.cell_h, page_w, self.cell_h)
    }

    /// The cell under physical point (`x`, `y`), clamped to the grid.
    pub fn cell_at(&self, x: i32, y: i32) -> (u16, u16) {
        let col = ((x - self.x).max(0) / self.cell_w).min(i32::from(self.cols) - 1);
        let row = ((y - self.y).max(0) / self.cell_h).min(i32::from(self.rows) - 1);
        (col as u16, row as u16)
    }
}

/// A selection in absolute line coordinates (`poly_vt`'s: 0 the first live
/// row, negative the history), from the anchor to the head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (i64, u16),
    pub head: (i64, u16),
}

impl Selection {
    /// Start and end, in order.
    pub fn ordered(&self) -> ((i64, u16), (i64, u16)) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    pub fn contains(&self, line: i64, col: u16) -> bool {
        let (a, b) = self.ordered();
        (line, col) >= a && (line, col) <= b
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// The header's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Copy,
    Paste,
    New,
}

/// Menu items.
const M_COPY: u32 = 1;
const M_PASTE: u32 = 2;
const M_SELECT_ALL: u32 = 3;
const M_CLEAR: u32 = 4;
const M_NEW: u32 = 5;

/// A key as `poly_vt` encodes it, if it goes to the shell.
pub fn vt_key(k: &KeyInput) -> Option<(keys::Key, KeyMods)> {
    let mods = KeyMods { shift: k.mods.shift, ctrl: k.mods.ctrl, alt: k.mods.alt };
    let key = match k.key {
        Some(Key::Enter) => keys::Key::Enter,
        Some(Key::NumpadEnter) => keys::Key::KpEnter,
        Some(Key::Tab) => keys::Key::Tab,
        Some(Key::Backspace) => keys::Key::Backspace,
        Some(Key::Escape) => keys::Key::Escape,
        Some(Key::ArrowUp) => keys::Key::Up,
        Some(Key::ArrowDown) => keys::Key::Down,
        Some(Key::ArrowLeft) => keys::Key::Left,
        Some(Key::ArrowRight) => keys::Key::Right,
        Some(Key::Home) => keys::Key::Home,
        Some(Key::End) => keys::Key::End,
        Some(Key::PageUp) => keys::Key::PageUp,
        Some(Key::PageDown) => keys::Key::PageDown,
        Some(Key::Insert) => keys::Key::Insert,
        Some(Key::Delete) => keys::Key::Delete,
        Some(f) if function_key(f).is_some() => keys::Key::F(function_key(f)?),
        _ => keys::Key::Char(k.ch?),
    };
    Some((key, mods))
}

fn function_key(k: Key) -> Option<u8> {
    use Key::*;
    Some(match k {
        F1 => 1,
        F2 => 2,
        F3 => 3,
        F4 => 4,
        F5 => 5,
        F6 => 6,
        F7 => 7,
        F8 => 8,
        F9 => 9,
        F10 => 10,
        F11 => 11,
        F12 => 12,
        _ => return None,
    })
}

/// The Terminal app.
pub struct TermApp {
    pub vt: Terminal,
    pub pty: Option<Pty>,
    /// Bytes for the shell that did not go yet.
    out: Vec<u8>,
    size: (i32, i32),
    scale: Scale,
    grid: Grid,
    /// The mono face's advance at the current scale, physical.
    advance: f32,
    pub selection: Option<Selection>,
    selecting: bool,
    focused: bool,
    hover: Option<Button>,
    pointer: (i32, i32),
    menu: Option<Menu>,
    /// Wheel movement not yet a whole line.
    wheel: f64,
    title: String,
    /// The mouse button held for a program that takes the mouse.
    mouse_held: Option<MouseButton>,
    last_mouse_cell: (u16, u16),
    /// Where the cursor was drawn (display row), to redraw it when it moves.
    drawn_cursor: Option<u16>,
    /// Display rows to draw at the next frame (all of them: `dirty_all`).
    dirty: Vec<bool>,
    dirty_all: bool,
    /// The shell ended: the window closes.
    pub ended: bool,
    /// What to run in a new window (this program).
    pub new_window: Option<Vec<String>>,
    /// Starts the shell: called once, at the first layout, with the grid's
    /// columns and rows and its size in pixels (see [`Pty`]'s `Launch`).
    pub launch: Option<Launcher>,
}

/// See [`TermApp::launch`].
pub type Launcher = alloc::boxed::Box<dyn FnOnce(u16, u16, u16, u16) -> Result<Pty, String>>;

impl core::fmt::Debug for TermApp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TermApp").field("grid", &self.grid).field("title", &self.title).finish_non_exhaustive()
    }
}

impl TermApp {
    pub fn new(pty: Option<Pty>, cols: u16, rows: u16) -> TermApp {
        TermApp {
            vt: Terminal::new(cols, rows, SCROLLBACK),
            pty,
            out: Vec::new(),
            size: (0, 0),
            scale: Scale::ONE,
            grid: Grid::default(),
            advance: 8.0,
            selection: None,
            selecting: false,
            focused: false,
            hover: None,
            pointer: (0, 0),
            menu: None,
            wheel: 0.0,
            title: "Terminal".into(),
            mouse_held: None,
            last_mouse_cell: (0, 0),
            drawn_cursor: None,
            dirty: Vec::new(),
            dirty_all: true,
            ended: false,
            new_window: None,
            launch: None,
        }
    }

    /// The header buttons' rectangles, logical.
    pub fn buttons(&self) -> [(Button, Rect); 3] {
        let w = self.size.0;
        let b = |i: i32| Rect::new(w - 12 - 28 * (3 - i) - 4 * (2 - i), 6, 28, 28);
        [(Button::Copy, b(0)), (Button::Paste, b(1)), (Button::New, b(2))]
    }

    fn button_at(&self, x: i32, y: i32) -> Option<Button> {
        self.buttons().iter().find(|(_, r)| r.contains(x, y)).map(|(b, _)| *b)
    }

    /// Queue `bytes` for the shell and send what goes now.
    pub fn send(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
        self.flush();
    }

    fn flush(&mut self) {
        let Some(pty) = &self.pty else {
            self.out.clear();
            return;
        };
        while !self.out.is_empty() {
            match pty.write(&self.out) {
                Ok(0) => break,
                Ok(n) => {
                    self.out.drain(..n);
                }
                Err(_) => {
                    self.out.clear();
                    break;
                }
            }
        }
    }

    /// Take output of the shell (or of a test).
    pub fn output(&mut self, bytes: &[u8], cx: &mut Cx) {
        self.vt.feed(bytes);
        let replies = self.vt.take_replies();
        if !replies.is_empty() {
            self.send(&replies);
        }
        if self.vt.take_title_changed() {
            let t = self.vt.title();
            self.title = if t.is_empty() { "Terminal".into() } else { t.into() };
            // For the boot test (and anyone reading the console).
            poly_rt::println!("poly-terminal: title {}", self.title);
            cx.title(self.title.clone());
            cx.damage(Rect::new(0, 0, self.size.0, HEADER));
        }
        let _ = self.vt.take_bell();
        self.damage(cx);
    }

    /// Tell the loop what of the screen changed.
    fn damage(&mut self, cx: &mut Cx) {
        match self.vt.take_damage() {
            Damage::All => {
                self.dirty_all = true;
                cx.damage(Rect::new(0, HEADER, self.size.0, self.size.1 - HEADER));
            }
            Damage::Rows(rows) => {
                let s = self.scale;
                if self.dirty.len() < rows.len() {
                    self.dirty.resize(rows.len(), false);
                }
                for (r, _) in rows.iter().enumerate().filter(|(_, d)| **d) {
                    self.dirty[r] = true;
                    let p = self.grid.row_rect(r as u16, self.size.0 * s);
                    cx.damage(s.rect_to_logical_out(p));
                }
            }
        }
        // The cursor's old row, if it moved off it.
        if let Some(r) = self.drawn_cursor.take() {
            let s = self.scale;
            let p = self.grid.row_rect(r, self.size.0 * s);
            cx.damage(s.rect_to_logical_out(p));
        }
    }

    fn redraw_screen(&mut self, cx: &mut Cx) {
        self.dirty_all = true;
        cx.damage(Rect::new(0, HEADER, self.size.0, self.size.1 - HEADER));
    }

    fn relayout(&mut self, ui: Option<&Ui>) {
        if let Some(ui) = ui {
            self.advance = ui.text.measure("M", mono(self.scale));
        }
        let g = Grid::new(self.size.0, self.size.1, self.scale, self.advance);
        if (g.cols, g.rows) != (self.vt.cols(), self.vt.rows()) {
            self.vt.resize(g.cols, g.rows);
            if let Some(p) = &self.pty {
                // A shell that is resized (`SIGWINCH`) may redraw its prompt:
                // the boot test checks none happens while the shell starts.
                poly_rt::println!("poly-terminal-pty: resized {}x{} (shell running)", g.cols, g.rows);
                let px = |n: i32| u16::try_from(n).unwrap_or(u16::MAX);
                p.resize(g.cols, g.rows, px(i32::from(g.cols) * g.cell_w), px(i32::from(g.rows) * g.cell_h));
            }
            self.selection = None;
        }
        self.grid = g;
        // Only with the real cell size (`ui`): a shell that started before it
        // would be resized, redraw its prompt (`SIGWINCH`) and show it twice.
        if ui.is_some() && self.size.0 > 0 && self.size.1 > 0 {
            self.start_shell();
        }
    }

    /// Start the shell on a terminal of the grid's size, once.
    fn start_shell(&mut self) {
        let Some(launch) = self.launch.take() else { return };
        let px = |n: i32| u16::try_from(n).unwrap_or(u16::MAX);
        let g = &self.grid;
        match launch(g.cols, g.rows, px(i32::from(g.cols) * g.cell_w), px(i32::from(g.rows) * g.cell_h)) {
            Ok(pty) => self.pty = Some(pty),
            Err(e) => crate::message(self, &e),
        }
    }

    /// The selection's text.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection.filter(|s| !s.is_empty())?;
        let (a, b) = sel.ordered();
        Some(self.vt.text(a, b))
    }

    fn copy(&self, cx: &mut Cx) {
        if let Some(t) = self.selected_text() {
            cx.copy(t);
        }
    }

    /// Absolute coordinates of the cell under logical (`x`, `y`).
    fn abs_cell(&self, x: i32, y: i32) -> (i64, u16) {
        let s = self.scale;
        let (col, row) = self.grid.cell_at(x * s, y * s);
        (self.vt.display_to_absolute(row), col)
    }

    /// A word around absolute (`line`, `col`): the run of non-blank,
    /// non-separator characters.
    fn word_at(&self, line: i64, col: u16) -> Selection {
        let row = line - self.vt.display_to_absolute(0);
        if row < 0 || row >= i64::from(self.vt.rows()) {
            return Selection { anchor: (line, col), head: (line, col) };
        }
        let cells = self.vt.visible_row(row as u16);
        let is_word = |c: &Cell| !c.ch.is_whitespace() && !"\"'`()[]{}<>|;,".contains(c.ch);
        let mut a = usize::from(col).min(cells.len().saturating_sub(1));
        let mut b = a;
        if cells.get(a).is_some_and(is_word) {
            while a > 0 && is_word(&cells[a - 1]) {
                a -= 1;
            }
            while b + 1 < cells.len() && is_word(&cells[b + 1]) {
                b += 1;
            }
        }
        Selection { anchor: (line, a as u16), head: (line, b as u16) }
    }

    fn program_wants_mouse(&self, shift: bool) -> bool {
        self.vt.modes().mouse != MouseMode::None && !shift
    }

    fn mouse_report(&mut self, action: MouseAction, x: i32, y: i32, mods: poly_app::Mods) {
        let s = self.scale;
        let (col, row) = self.grid.cell_at(x * s, y * s);
        let ev = MouseEvent { action, col, row, mods: KeyMods { shift: mods.shift, ctrl: mods.ctrl, alt: mods.alt } };
        let bytes = keys::mouse(ev, self.vt.modes());
        if !bytes.is_empty() {
            self.send(&bytes);
        }
        self.last_mouse_cell = (col, row);
    }

    fn key(&mut self, k: KeyInput, cx: &mut Cx) {
        if !k.pressed {
            return;
        }
        if self.menu.is_some() && k.key == Some(Key::Escape) {
            self.menu = None;
            cx.redraw();
            return;
        }
        let m = k.mods;
        if m.meta {
            return;
        }
        // The terminal's own keys.
        if m.ctrl && m.shift && !m.alt {
            match k.ch.map(|c| c.to_ascii_lowercase()) {
                Some('c') => return self.copy(cx),
                Some('v') => return cx.paste(),
                Some('n') => return self.open_new(),
                _ => {}
            }
        }
        if m.shift && !m.ctrl && !m.alt {
            match k.key {
                Some(Key::Insert) => return cx.paste(),
                Some(Key::PageUp) => {
                    self.vt.scroll_view(i64::from(self.vt.rows() / 2).max(1) as isize);
                    return self.damage(cx);
                }
                Some(Key::PageDown) => {
                    self.vt.scroll_view(-(i64::from(self.vt.rows() / 2).max(1) as isize));
                    return self.damage(cx);
                }
                _ => {}
            }
        }
        let Some((key, mods)) = vt_key(&k) else { return };
        let bytes = keys::encode(key, mods, self.vt.modes());
        if bytes.is_empty() {
            return;
        }
        if self.vt.view_offset() != 0 {
            self.vt.reset_view();
        }
        if self.selection.take().is_some() {
            self.redraw_screen(cx);
        }
        self.send(&bytes);
        self.damage(cx);
    }

    /// Another Terminal window: a new process of this program, with the
    /// environment the desktop gave this one.
    fn open_new(&mut self) {
        let argv: Vec<String> = alloc::vec![poly_shell::apps::TERMINAL.into()];
        let mut cmd = poly_rt::process::Command::new(&argv[0]);
        cmd.envs(poly_rt::env::vars().filter_map(|v| core::str::from_utf8(v).ok()))
            .stdin(poly_rt::process::Stdio::Null);
        // On the host there is no kernel to start it; the request is kept.
        let _ = cmd.spawn();
        self.new_window = Some(argv);
    }

    fn menu_item(&mut self, id: u32, cx: &mut Cx) {
        match id {
            M_COPY => self.copy(cx),
            M_PASTE => cx.paste(),
            M_SELECT_ALL => {
                let first = -(self.vt.scrollback_len() as i64);
                let last = i64::from(self.vt.rows()) - 1;
                self.selection = Some(Selection { anchor: (first, 0), head: (last, self.vt.cols() - 1) });
            }
            M_CLEAR => {
                // As `clear` does: the screen and the history.
                self.vt.feed(b"\x1b[H\x1b[2J\x1b[3J");
                self.send(b"\x0c");
                self.selection = None;
            }
            M_NEW => self.open_new(),
            _ => {}
        }
        cx.redraw();
    }

    fn button(&mut self, button: u32, pressed: bool, x: i32, y: i32, clicks: u32, cx: &mut Cx) {
        if let Some(menu) = self.menu.take() {
            if pressed && let Some(i) = menu.item_at(x, y) {
                let id = menu.items[i].id;
                self.menu_item(id, cx);
            }
            cx.redraw();
            return;
        }
        if y < HEADER {
            if pressed
                && button == poly_app::BTN_LEFT
                && let Some(b) = self.button_at(x, y)
            {
                match b {
                    Button::Copy => self.copy(cx),
                    Button::Paste => cx.paste(),
                    Button::New => self.open_new(),
                }
            }
            return;
        }
        let mods = poly_app::Mods::default();
        if self.program_wants_mouse(false) {
            let b = match button {
                poly_app::BTN_LEFT => MouseButton::Left,
                poly_app::BTN_MIDDLE => MouseButton::Middle,
                _ => MouseButton::Right,
            };
            self.mouse_held = pressed.then_some(b);
            let action = if pressed { MouseAction::Press(b) } else { MouseAction::Release(b) };
            self.mouse_report(action, x, y, mods);
            return;
        }
        match (button, pressed) {
            (poly_app::BTN_LEFT, true) => {
                let at = self.abs_cell(x, y);
                self.selection = Some(match clicks {
                    1 => Selection { anchor: at, head: at },
                    2 => self.word_at(at.0, at.1),
                    _ => Selection { anchor: (at.0, 0), head: (at.0, self.vt.cols() - 1) },
                });
                self.selecting = clicks == 1;
                self.redraw_screen(cx);
            }
            (poly_app::BTN_LEFT, false) => {
                self.selecting = false;
                if self.selection.is_some_and(|s| s.is_empty()) {
                    self.selection = None;
                    self.redraw_screen(cx);
                }
            }
            (poly_app::BTN_MIDDLE, true) => cx.paste(),
            (poly_app::BTN_RIGHT, true) => {
                let has = self.selected_text().is_some();
                let items = alloc::vec![
                    MenuItem::new(M_COPY, "Copy", "Ctrl+Shift+C", has),
                    MenuItem::new(M_PASTE, "Paste", "Ctrl+Shift+V", true),
                    MenuItem::separator(),
                    MenuItem::new(M_SELECT_ALL, "Select all", "", true),
                    MenuItem::new(M_CLEAR, "Clear", "", true),
                    MenuItem::separator(),
                    MenuItem::new(M_NEW, "New window", "Ctrl+Shift+N", true),
                ];
                self.menu = Some(Menu::new(items, x, y, self.size.0, self.size.1));
                cx.redraw();
            }
            _ => {}
        }
    }

    fn motion(&mut self, x: i32, y: i32, cx: &mut Cx) {
        self.pointer = (x, y);
        if let Some(m) = &mut self.menu {
            let h = m.item_at(x, y);
            if h != m.hover {
                m.hover = h;
                cx.redraw();
            }
            return;
        }
        let hover = if y < HEADER { self.button_at(x, y) } else { None };
        if hover != self.hover {
            self.hover = hover;
            cx.damage(Rect::new(0, 0, self.size.0, HEADER));
        }
        cx.cursor(if y >= HEADER { CursorShape::Text } else { CursorShape::Default });
        if y >= HEADER && self.program_wants_mouse(false) {
            let s = self.scale;
            let cell = self.grid.cell_at(x * s, y * s);
            let modes = self.vt.modes();
            let report = match modes.mouse {
                MouseMode::Any => true,
                MouseMode::Drag => self.mouse_held.is_some(),
                _ => false,
            };
            if report && cell != self.last_mouse_cell {
                self.mouse_report(MouseAction::Motion(self.mouse_held), x, y, poly_app::Mods::default());
            }
            return;
        }
        if self.selecting {
            let at = self.abs_cell(x, y);
            if let Some(sel) = &mut self.selection
                && sel.head != at
            {
                sel.head = at;
                self.redraw_screen(cx);
            }
        }
    }

    fn scroll(&mut self, dy: f64, cx: &mut Cx) {
        // 15 logical pixels a notch; three lines a notch.
        self.wheel += dy / 5.0;
        let lines = self.wheel as i64;
        if lines == 0 {
            return;
        }
        self.wheel -= lines as f64;
        if self.program_wants_mouse(false) {
            let b = if lines < 0 { MouseButton::WheelUp } else { MouseButton::WheelDown };
            let (x, y) = self.pointer;
            for _ in 0..lines.unsigned_abs().min(10) {
                self.mouse_report(MouseAction::Press(b), x, y, poly_app::Mods::default());
            }
        } else if self.vt.modes().alt_screen {
            let key = if lines < 0 { keys::Key::Up } else { keys::Key::Down };
            let bytes = keys::encode(key, KeyMods::default(), self.vt.modes());
            for _ in 0..lines.unsigned_abs().min(30) {
                self.send(&bytes);
            }
        } else {
            self.vt.scroll_view(-(lines as isize));
            self.damage(cx);
        }
    }

    /// Draw display row `r` (physical pixels).
    fn draw_row(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect, r: u16, light: bool) {
        let g = self.grid;
        let s = self.scale;
        let page_w = self.size.0 * s;
        let row_rect = g.row_rect(r, page_w);
        let clip = clip.intersect(&row_rect);
        if clip.is_empty() {
            return;
        }
        poly_shell::draw::fill(fb, clip, row_rect, pal().s1, 255);
        let cells = self.vt.visible_row(r);
        let line = self.vt.display_to_absolute(r);
        let reverse = self.vt.modes().reverse_video;
        let st = mono(s);
        let baseline = ui.text.centre(row_rect.y, g.cell_h, st);
        let cursor = self.vt.cursor_on_display().filter(|_| self.vt.cursor().visible);
        let mut buf = [0u8; 4];
        for (c, cell) in cells.iter().enumerate().take(usize::from(self.vt.cols())) {
            if cell.width == 0 {
                continue;
            }
            let x = g.x + c as i32 * g.cell_w;
            let w = g.cell_w * i32::from(cell.width.max(1));
            let cr = Rect::new(x, row_rect.y, w, g.cell_h);
            let (mut fg, bg) = cell_colors(cell, light, reverse);
            if bg != pal().s1 {
                poly_shell::draw::fill(fb, clip, cr, bg, 255);
            }
            if self.selection.is_some_and(|sel| !sel.is_empty() && sel.contains(line, c as u16)) {
                poly_shell::draw::fill(fb, clip, cr, pal().ink, 70);
            }
            let is_cursor = cursor == Some((r, c as u16));
            if is_cursor {
                match (self.vt.cursor().shape, self.focused) {
                    (VtCursor::Block, true) => {
                        poly_shell::draw::fill(fb, clip, cr, pal().ink_primary, 255);
                        fg = pal().s1;
                    }
                    (VtCursor::Block, false) => {
                        poly_shell::draw::rounded_border(fb, clip, cr, 0, s.line(1), pal().ink_primary, 200);
                    }
                    (VtCursor::Underline, _) => {
                        let u = Rect::new(x, row_rect.y + g.cell_h - 2 * s, w, 2 * s);
                        poly_shell::draw::fill(fb, clip, u, pal().ink_primary, 255);
                    }
                    (VtCursor::Bar, _) => {
                        poly_shell::draw::fill(
                            fb,
                            clip,
                            Rect::new(x, row_rect.y, 2 * s, g.cell_h),
                            pal().ink_primary,
                            255,
                        );
                    }
                }
            }
            if cell.ch != ' ' && !cell.attrs.contains(Attrs::HIDDEN) {
                let text = cell.ch.encode_utf8(&mut buf);
                ui.text.draw_style(fb, clip, x as f32, baseline, x + w, text, st, fg, 255);
                if cell.attrs.contains(Attrs::BOLD) {
                    // Geist Mono has one weight here: bold is drawn twice,
                    // a pixel apart.
                    ui.text.draw_style(
                        fb,
                        clip,
                        (x + s.line(1) / 2 + 1) as f32 - 0.5,
                        baseline,
                        x + w,
                        text,
                        st,
                        fg,
                        160,
                    );
                }
            }
            if cell.attrs.contains(Attrs::UNDERLINE) {
                poly_shell::draw::fill(fb, clip, Rect::new(x, baseline + 2 * s, w, s.line(1)), fg, 255);
            }
            if cell.attrs.contains(Attrs::STRIKE) {
                poly_shell::draw::fill(fb, clip, Rect::new(x, baseline - g.cell_h / 4, w, s.line(1)), fg, 255);
            }
        }
    }

    /// The header: the icon, the title, the buttons, the hairline.
    fn draw_header(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect) {
        let (w, _) = self.size;
        ui.fill(fb, clip, Rect::new(0, 0, w, HEADER), pal().s0, 255);
        ui.icon(fb, clip, lucide::SQUARE_TERMINAL, 14, 12, 16, pal().quiet, 255);
        let max = self.buttons()[0].1.x - 12;
        let title = ui.fit(&self.title, poly_app::ui::DENSE_MEDIUM, max - 40);
        ui.text(fb, clip, 40, 0, HEADER, max, &title, poly_app::ui::DENSE_MEDIUM, pal().ink_primary);
        for (b, r) in self.buttons() {
            let (icon, enabled) = match b {
                Button::Copy => (lucide::COPY, self.selected_text().is_some()),
                Button::Paste => (lucide::CLIPBOARD_PASTE, true),
                Button::New => (lucide::PLUS, true),
            };
            ui.icon_button(fb, clip, r, icon, 15, self.hover == Some(b), enabled);
        }
        ui.fill(fb, clip, Rect::new(0, HEADER - 1, w, 1), pal().ink, pal().hairline);
    }
}

impl App for TermApp {
    fn event(&mut self, ev: AppEvent, cx: &mut Cx) {
        match ev {
            AppEvent::Resized { width, height, scale } => {
                self.size = (width, height);
                if scale != self.scale {
                    self.scale = scale;
                    // The advance is measured again at the next draw.
                    self.advance = 0.0;
                }
                cx.redraw();
            }
            AppEvent::Key(k) => self.key(k, cx),
            AppEvent::Motion { x, y } => self.motion(x, y, cx),
            AppEvent::Leave => {
                if self.hover.take().is_some() {
                    cx.damage(Rect::new(0, 0, self.size.0, HEADER));
                }
            }
            AppEvent::Button { button, pressed, x, y, clicks, .. } => self.button(button, pressed, x, y, clicks, cx),
            AppEvent::Scroll { dy } => self.scroll(dy, cx),
            AppEvent::Paste(text) => {
                let bytes = keys::paste(&text, self.vt.modes());
                self.vt.reset_view();
                self.send(&bytes);
                self.damage(cx);
            }
            AppEvent::Focus(on) => {
                self.focused = on;
                let bytes = keys::focus(on, self.vt.modes());
                if !bytes.is_empty() {
                    self.send(&bytes);
                }
                if let Some((r, _)) = self.vt.cursor_on_display() {
                    let s = self.scale;
                    let p = self.grid.row_rect(r, self.size.0 * s);
                    cx.damage(s.rect_to_logical_out(p));
                }
            }
            AppEvent::Theme => cx.redraw(),
        }
    }

    fn draw(&mut self, fb: &mut dyn Framebuffer, ui: &Ui, full: bool) -> Option<Vec<Rect>> {
        if self.advance <= 0.0 || full {
            self.relayout(Some(ui));
        }
        let light = poly_shell::theme::is_light();
        let s = self.scale;
        let all = Rect::new(0, 0, self.size.0 * s, self.size.1 * s);
        // What changed since the last frame.
        let damage = self.vt.take_damage();
        let all_rows = full || self.dirty_all || matches!(damage, Damage::All);
        let rows: Vec<u16> = (0..self.vt.rows())
            .filter(|&r| {
                all_rows
                    || damage.contains(r)
                    || self.dirty.get(usize::from(r)).copied().unwrap_or(false)
                    || self.drawn_cursor == Some(r)
            })
            .collect();
        self.dirty_all = false;
        self.dirty.clear();
        let mut drawn = Vec::new();
        if full {
            self.draw_header(ui, fb, all);
            ui.fill(fb, all, Rect::new(0, HEADER, self.size.0, self.size.1 - HEADER), pal().s1, 255);
        } else {
            // The header may have changed (title, hover): it is cheap.
            self.draw_header(ui, fb, all);
            drawn.push(Rect::new(0, 0, self.size.0, HEADER));
        }
        for r in rows {
            self.draw_row(ui, fb, all, r, light);
            let p = self.grid.row_rect(r, all.width);
            drawn.push(s.rect_to_logical_out(p));
        }
        self.drawn_cursor = self.vt.cursor_on_display().map(|(r, _)| r);
        if let Some(m) = &self.menu {
            m.draw(ui, fb, all);
            drawn.push(m.rect().grow(24));
        }
        if full { None } else { Some(drawn) }
    }

    fn handles(&self) -> Vec<(u32, u16)> {
        let Some(p) = &self.pty else { return Vec::new() };
        let mut ev = poly_rt::poll::READABLE;
        if !self.out.is_empty() {
            ev |= poly_rt::poll::WRITABLE;
        }
        alloc::vec![(p.master.handle(), ev)]
    }

    fn ready(&mut self, _handle: u32, revents: u16, cx: &mut Cx) {
        if revents & poly_rt::poll::WRITABLE != 0 {
            self.flush();
        }
        let mut buf = alloc::vec![0u8; 64 * 1024];
        let mut total = 0;
        let mut gone = false;
        // At most 1 MiB a turn, so the window keeps answering.
        while total < (1 << 20) {
            let Some(p) = &self.pty else { break };
            match p.read(&mut buf) {
                Some(0) => break,
                Some(n) => {
                    total += n;
                    let chunk = buf[..n].to_vec();
                    self.output(&chunk, cx);
                }
                None => {
                    gone = true;
                    break;
                }
            }
        }
        if gone || (revents & poly_rt::poll::HANGUP != 0 && total == 0) {
            self.ended = true;
            cx.quit();
        }
    }

    fn close(&mut self, cx: &mut Cx) {
        if let Some(p) = &self.pty {
            p.hang_up();
        }
        cx.quit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_app::Mods;
    use poly_shell::draw::Text;
    use poly_wayland::compose::MemoryFramebuffer;

    fn ui() -> Ui {
        Ui { text: Text::new(None), scale: Scale::ONE }
    }

    fn app(w: i32, h: i32) -> TermApp {
        let mut t = TermApp::new(None, 80, 24);
        let mut cx = Cx::default();
        t.event(AppEvent::Resized { width: w, height: h, scale: Scale::ONE }, &mut cx);
        t
    }

    #[test]
    fn the_grid_fills_the_page_inside_its_padding() {
        let g = Grid::new(800, 520, Scale::from_int(1), 7.8);
        assert_eq!((g.cell_w, g.cell_h), (8, 18));
        assert_eq!((g.cols, g.rows), ((800 - 24) as u16 / 8, ((520 - 40 - 24) / 18) as u16));
        let g2 = Grid::new(800, 520, Scale::from_int(2), 15.6);
        assert_eq!((g2.cell_w, g2.cell_h), (16, 35));
        assert_eq!(g2.cell_at(g2.x + 16 * 3 + 1, g2.y + 35 * 2 + 1), (3, 2));
        assert_eq!(g2.cell_at(0, 0), (0, 0), "clamped");
    }

    #[test]
    fn output_draws_and_resizes_the_emulator() {
        let mut t = app(640, 480);
        let mut fb = MemoryFramebuffer::new(640, 480);
        t.draw(&mut fb, &ui(), true);
        let g = t.grid;
        assert_eq!((t.vt.cols(), t.vt.rows()), (g.cols, g.rows));
        let mut cx = Cx::default();
        t.output(b"\x1b[41mhi\x1b[0m", &mut cx);
        let drawn = t.draw(&mut fb, &ui(), false).expect("partial");
        assert!(drawn.iter().any(|r| r.y >= HEADER), "{drawn:?}");
        // The red ground of "h" is there.
        let red = ansi(false)[1];
        assert_eq!(fb.pixel((g.x + 1) as u32, (g.y + 1) as u32), Some(red));
        // The page around it is surface 1.
        assert_eq!(fb.pixel(2, (g.y + 2) as u32), Some(pal().s1));
        // The header is the ground.
        assert_eq!(fb.pixel(5, 5), Some(pal().s0));
    }

    /// A launcher that records what it was called with and fails (there is
    /// no pseudo-terminal on the host).
    type Calls = std::rc::Rc<std::cell::RefCell<Vec<(u16, u16, u16, u16)>>>;

    fn recording_launcher(calls: &Calls) -> Launcher {
        let calls = std::rc::Rc::clone(calls);
        alloc::boxed::Box::new(move |c, r, w, h| {
            calls.borrow_mut().push((c, r, w, h));
            Err("test: no pty here".into())
        })
    }

    #[test]
    fn the_shell_starts_at_the_first_layout_with_the_windows_size() {
        // A shell started at a placeholder size is resized when the window's
        // size arrives, redraws its prompt on SIGWINCH and shows it twice
        // (BusyBox's ash does). So the terminal starts it only once the grid
        // is known, with that grid.
        let calls = std::rc::Rc::default();
        let mut t = TermApp::new(None, 80, 24);
        t.launch = Some(recording_launcher(&calls));
        let mut cx = Cx::default();
        let mut fb = MemoryFramebuffer::new(640, 480);
        assert!(calls.borrow().is_empty(), "not at creation");
        // The size arrives, but the cell size does not yet: still waiting.
        t.event(AppEvent::Resized { width: 640, height: 480, scale: Scale::ONE }, &mut cx);
        assert!(calls.borrow().is_empty(), "not before the first draw measures the cells");
        t.draw(&mut fb, &ui(), true);
        let g = t.grid;
        assert_eq!((g.cols, g.rows), (t.vt.cols(), t.vt.rows()), "the emulator has the real size first");
        assert_eq!(
            *calls.borrow(),
            [(g.cols, g.rows, g.cols * g.cell_w as u16, g.rows * g.cell_h as u16)],
            "started once, with the grid's columns, rows and pixels"
        );
        assert_ne!((g.cols, g.rows), (80, 24), "a size other than the placeholder");
        // Later layouts do not start it again.
        t.event(AppEvent::Resized { width: 700, height: 500, scale: Scale::ONE }, &mut cx);
        t.draw(&mut fb, &ui(), true);
        assert_eq!(calls.borrow().len(), 1);
    }

    #[test]
    fn a_shell_that_cannot_start_says_so_in_the_window() {
        let calls = std::rc::Rc::default();
        let mut t = app(640, 480);
        t.launch = Some(recording_launcher(&calls));
        let mut fb = MemoryFramebuffer::new(640, 480);
        t.draw(&mut fb, &ui(), true);
        assert!(t.pty.is_none());
        assert!(t.vt.text((t.vt.display_to_absolute(0), 0), (t.vt.display_to_absolute(5), 79)).contains("no pty here"));
    }

    #[test]
    fn colours_follow_the_theme_and_attributes() {
        let mut c = Cell::BLANK;
        assert_eq!(cell_colors(&c, false, false), (pal().ink_primary, pal().s1));
        c.fg = Color::Indexed(1);
        c.attrs = Attrs::BOLD;
        assert_eq!(cell_colors(&c, false, false).0, ansi(false)[9], "bold is bright");
        c.attrs = Attrs::INVERSE;
        assert_eq!(cell_colors(&c, false, false), (pal().s1, ansi(false)[1]));
        assert_eq!(rgb(Color::Rgb(1, 2, 3), true, false), 0x01_0203);
        assert_eq!(rgb(Color::Indexed(16), true, false), 0);
        assert_eq!(ansi(true)[1], 0xdc_2626);
    }

    #[test]
    fn keys_map_to_xterm() {
        let k = |key: Option<Key>, ch: Option<char>, mods: Mods| KeyInput {
            code: 0,
            key,
            ch,
            mods,
            pressed: true,
            repeat: false,
        };
        let none = Mods::default();
        let ctrl = Mods { ctrl: true, ..none };
        let (key, m) = vt_key(&k(Some(Key::C), Some('c'), ctrl)).unwrap();
        assert_eq!(keys::encode(key, m, &poly_vt::Modes::default()), [3]);
        let (key, m) = vt_key(&k(Some(Key::ArrowUp), None, none)).unwrap();
        assert_eq!(keys::encode(key, m, &poly_vt::Modes::default()), b"\x1b[A");
        let (key, _) = vt_key(&k(Some(Key::F5), None, none)).unwrap();
        assert_eq!(key, keys::Key::F(5));
        assert!(vt_key(&k(Some(Key::ShiftLeft), None, none)).is_none());
    }

    #[test]
    fn selections_copy_text_and_clicks_pick_words() {
        let mut t = app(640, 480);
        let mut fb = MemoryFramebuffer::new(640, 480);
        t.draw(&mut fb, &ui(), true);
        let mut cx = Cx::default();
        t.output(b"echo hello world\r\n", &mut cx);
        let g = t.grid;
        // Double click on "hello" (columns 5..9 of row 0).
        let (x, y) = (g.x + 6 * g.cell_w + 2, g.y + 3);
        t.event(
            AppEvent::Button { button: poly_app::BTN_LEFT, pressed: true, x, y, clicks: 2, mods: Mods::default() },
            &mut cx,
        );
        t.event(
            AppEvent::Button { button: poly_app::BTN_LEFT, pressed: false, x, y, clicks: 2, mods: Mods::default() },
            &mut cx,
        );
        assert_eq!(t.selected_text().as_deref(), Some("hello"));
        // A drag from "echo" to "world".
        t.event(
            AppEvent::Button {
                button: poly_app::BTN_LEFT,
                pressed: true,
                x: g.x + 1,
                y,
                clicks: 1,
                mods: Mods::default(),
            },
            &mut cx,
        );
        t.event(AppEvent::Motion { x: g.x + 15 * g.cell_w + 1, y }, &mut cx);
        t.event(
            AppEvent::Button {
                button: poly_app::BTN_LEFT,
                pressed: false,
                x: 0,
                y: 0,
                clicks: 1,
                mods: Mods::default(),
            },
            &mut cx,
        );
        assert_eq!(t.selected_text().as_deref(), Some("echo hello world"));
        // Typing drops the selection.
        t.event(
            AppEvent::Key(KeyInput {
                code: 30,
                key: Some(Key::A),
                ch: Some('a'),
                mods: Mods::default(),
                pressed: true,
                repeat: false,
            }),
            &mut cx,
        );
        assert!(t.selection.is_none());
    }

    #[test]
    fn titles_from_the_shell_name_the_window() {
        let mut t = app(640, 480);
        let mut cx = Cx::default();
        t.output(b"\x1b]0;root@poly: /tmp\x07", &mut cx);
        assert_eq!(t.title, "root@poly: /tmp");
    }

    #[test]
    fn the_menu_opens_on_the_right_button() {
        let mut t = app(640, 480);
        let mut cx = Cx::default();
        t.event(
            AppEvent::Button {
                button: poly_app::BTN_RIGHT,
                pressed: true,
                x: 100,
                y: 100,
                clicks: 1,
                mods: Mods::default(),
            },
            &mut cx,
        );
        let m = t.menu.clone().expect("a menu");
        assert!(!m.items[0].enabled, "nothing to copy");
        let mut fb = MemoryFramebuffer::new(640, 480);
        t.draw(&mut fb, &ui(), true);
        // Clear, from the menu.
        let clear = m.items.iter().position(|i| i.id == M_CLEAR).unwrap();
        let mut y = m.at.1 + poly_app::ui::MENU_PAD;
        for it in &m.items[..clear] {
            y += if it.separator { poly_app::ui::MENU_SEPARATOR } else { poly_app::ui::MENU_ROW };
        }
        t.event(
            AppEvent::Button {
                button: poly_app::BTN_LEFT,
                pressed: true,
                x: m.at.0 + 20,
                y: y + 5,
                clicks: 1,
                mods: Mods::default(),
            },
            &mut cx,
        );
        assert!(t.menu.is_none());
    }
}
