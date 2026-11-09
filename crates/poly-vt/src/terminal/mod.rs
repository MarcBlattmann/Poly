//! The terminal: a screen of cells, the cursor, modes, scrollback, and
//! what the program's output does to them.
//!
//! # Screens
//!
//! There are two screens of `rows` x `cols` cells. The primary one has a
//! scrollback: lines that scroll off its top (with the scroll region's top
//! margin at the first row) go there, the oldest dropped past the limit
//! given to [`Terminal::new`]. The alternate screen (modes `?47`, `?1047`,
//! `?1049`, used by full-screen programs) has none. Each screen has its own
//! saved cursor (`ESC 7` / `ESC 8`).
//!
//! # Writing
//!
//! Characters go at the cursor in the current pen (colours and style) and
//! move it right. Writing in the last column leaves the cursor there with a
//! wrap pending (the VT "last column flag"): the next character first moves
//! to the start of the next line, scrolling at the bottom margin, and marks
//! the line as soft-wrapped. A wide character that does not fit in the last
//! column wraps early, leaving a blank. With autowrap off (`?7l`) the last
//! column is overwritten instead. Insert mode (`4h`) shifts the rest of the
//! line right. Erasing fills with blanks in the current background colour
//! (xterm's "background colour erase").
//!
//! LF, VT and FF move down a line (scrolling at the bottom margin) without
//! returning, unless newline mode (`20h`) is set; CR returns; BS moves left;
//! HT moves to the next tab stop (every 8 columns, `ESC H` / `CSI g` to
//! change); BEL rings ([`Terminal::take_bell`]).
//!
//! The control sequences carried out are listed in [`csi`](self) and on
//! [`Terminal::feed`]; everything else is ignored.
//!
//! # Viewing
//!
//! [`Terminal::visible_row`] gives the rows as displayed: with the view
//! scrolled `n` lines into the scrollback ([`Terminal::scroll_view`]), the
//! top `n` rows show history. New output while scrolled back keeps the view
//! on the same lines. [`Terminal::take_damage`] says which displayed rows
//! changed since the last call. Selections use *absolute* line numbers: 0
//! is the first screen row, -1 the newest scrollback line.

mod csi;
mod edit;
mod screen;
mod view;

use crate::cell::{Cell, Pen};
use crate::charset::Charset;
use crate::parser::{Csi, Parser, Perform};
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
pub use view::Damage;

use screen::Line;

/// The most columns a terminal can have.
pub const MAX_COLS: u16 = 4096;
/// The most rows a terminal can have.
pub const MAX_ROWS: u16 = 2048;
/// The most characters of a window title kept.
const MAX_TITLE: usize = 1024;

/// How the cursor is drawn (`CSI Ps SP q`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

/// The cursor as the renderer should draw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cursor {
    /// Screen row (0-based). With the view scrolled back by `n` it shows on
    /// display row `row + n`, if that is still on the display.
    pub row: u16,
    pub col: u16,
    /// `?25`.
    pub visible: bool,
    pub shape: CursorShape,
    /// `?12`, or a blinking `DECSCUSR` style.
    pub blink: bool,
}

/// Which mouse events the program asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseMode {
    /// None: the terminal uses the mouse itself (selection).
    #[default]
    None,
    /// Button presses and releases (`?9`, `?1000`).
    Press,
    /// Also motion while a button is held (`?1002`).
    Drag,
    /// Also any motion (`?1003`).
    Any,
}

/// The terminal modes that matter outside the screen model (to encode
/// input, or to draw).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Modes {
    /// `?1` (DECCKM): cursor keys send `SS3 A` instead of `CSI A`.
    pub app_cursor: bool,
    /// `ESC =` / `?66` (DECKPAM): the keypad sends `SS3` sequences.
    pub app_keypad: bool,
    /// `?2004`: pasted text is bracketed by `CSI 200~` / `CSI 201~`.
    pub bracketed_paste: bool,
    /// The alternate screen is shown.
    pub alt_screen: bool,
    /// `?7` (DECAWM), on by default.
    pub autowrap: bool,
    /// `4` (IRM).
    pub insert: bool,
    /// `?6` (DECOM): cursor addressing relative to the scroll region.
    pub origin: bool,
    pub mouse: MouseMode,
    /// `?1006`: mouse reports in the SGR format.
    pub sgr_mouse: bool,
    /// `?1004`: report focus in and out.
    pub focus_events: bool,
    /// `?5` (DECSCNM): draw the whole screen in reverse video.
    pub reverse_video: bool,
    /// `20` (LNM): LF also returns, and Enter sends CR LF.
    pub newline: bool,
}

impl Default for Modes {
    fn default() -> Modes {
        Modes {
            app_cursor: false,
            app_keypad: false,
            bracketed_paste: false,
            alt_screen: false,
            autowrap: true,
            insert: false,
            origin: false,
            mouse: MouseMode::None,
            sgr_mouse: false,
            focus_events: false,
            reverse_video: false,
            newline: false,
        }
    }
}

/// Where the cursor is and what it writes with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CursorState {
    row: usize,
    col: usize,
    /// Written in the last column; the next character wraps first.
    pending_wrap: bool,
}

/// What `ESC 7` saves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Saved {
    cur: CursorState,
    pen: Pen,
    charsets: [Charset; 2],
    gl: usize,
    origin: bool,
}

/// A terminal emulator: feed it the program's output, draw its cells,
/// send the program what [`Terminal::take_replies`] and
/// [`crate::keys`] produce.
#[derive(Debug)]
pub struct Terminal {
    cols: usize,
    rows: usize,
    /// The screen being shown.
    screen: Vec<Line>,
    /// The other screen (the primary while the alternate one is shown).
    other: Vec<Line>,
    scrollback: VecDeque<Line>,
    scrollback_cap: usize,
    /// Lines that ever went into the scrollback.
    scrolled: u64,
    view_offset: usize,

    cur: CursorState,
    pen: Pen,
    charsets: [Charset; 2],
    /// Which of G0 / G1 is in use.
    gl: usize,
    /// Saved cursors of the primary and the alternate screen.
    saved: [Option<Saved>; 2],
    /// The scroll region, inclusive.
    top: usize,
    bottom: usize,
    tabs: Vec<bool>,
    modes: Modes,
    cursor_visible: bool,
    cursor_shape: CursorShape,
    cursor_blink: bool,
    last_char: Option<char>,

    title: String,
    title_changed: bool,
    cwd: String,
    bell: bool,
    replies: Vec<u8>,
    default_fg: (u8, u8, u8),
    default_bg: (u8, u8, u8),

    damage_all: bool,
    dirty: Vec<bool>,
    last_cursor: Option<(usize, usize)>,

    parser: Parser,
}

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c % 8 == 0 && c != 0).collect()
}

impl Terminal {
    /// A terminal of `cols` x `rows` cells (each clamped to 1..=[`MAX_COLS`]
    /// / [`MAX_ROWS`]) keeping up to `scrollback` lines of history.
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Terminal {
        let cols = usize::from(cols.clamp(1, MAX_COLS));
        let rows = usize::from(rows.clamp(1, MAX_ROWS));
        Terminal {
            cols,
            rows,
            screen: vec![Line::blank(cols, Cell::BLANK); rows],
            other: vec![Line::blank(cols, Cell::BLANK); rows],
            scrollback: VecDeque::new(),
            scrollback_cap: scrollback,
            scrolled: 0,
            view_offset: 0,
            cur: CursorState::default(),
            pen: Pen::default(),
            charsets: [Charset::Ascii; 2],
            gl: 0,
            saved: [None; 2],
            top: 0,
            bottom: rows - 1,
            tabs: default_tabs(cols),
            modes: Modes::default(),
            cursor_visible: true,
            cursor_shape: CursorShape::Block,
            cursor_blink: false,
            last_char: None,
            title: String::new(),
            title_changed: false,
            cwd: String::new(),
            bell: false,
            replies: Vec::new(),
            default_fg: (0xff, 0xff, 0xff),
            default_bg: (0, 0, 0),
            damage_all: true,
            dirty: vec![false; rows],
            last_cursor: None,
            parser: Parser::default(),
        }
    }

    /// Carry out output of the program: text, control characters and
    /// escape sequences. UTF-8 is decoded across calls (a character or a
    /// sequence may be split anywhere); malformed UTF-8 shows as U+FFFD.
    /// Never panics, whatever the bytes.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut parser = core::mem::take(&mut self.parser);
        parser.feed(bytes, self);
        self.parser = parser;
    }

    /// Bytes the terminal must send back to the program (answers to status
    /// and identification requests), emptied by the call.
    pub fn take_replies(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.replies)
    }

    /// Columns.
    pub fn cols(&self) -> u16 {
        self.cols as u16
    }

    /// Rows.
    pub fn rows(&self) -> u16 {
        self.rows as u16
    }

    /// The cursor.
    pub fn cursor(&self) -> Cursor {
        Cursor {
            row: self.cur.row as u16,
            col: self.cur.col as u16,
            visible: self.cursor_visible,
            shape: self.cursor_shape,
            blink: self.cursor_blink,
        }
    }

    /// The modes.
    pub fn modes(&self) -> &Modes {
        &self.modes
    }

    /// The window title (OSC 0 / OSC 2).
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether the title was set since the last call.
    pub fn take_title_changed(&mut self) -> bool {
        core::mem::take(&mut self.title_changed)
    }

    /// The working directory the shell reported (OSC 7, a `file://` URL),
    /// or "".
    pub fn working_directory(&self) -> &str {
        &self.cwd
    }

    /// Whether BEL came since the last call.
    pub fn take_bell(&mut self) -> bool {
        core::mem::take(&mut self.bell)
    }

    /// The default foreground and background colours, for answering
    /// colour queries (OSC 10 / OSC 11 with `?`).
    pub fn set_default_colors(&mut self, fg: (u8, u8, u8), bg: (u8, u8, u8)) {
        self.default_fg = fg;
        self.default_bg = bg;
    }

    /// Change the size. Content is kept: shrinking moves lines above the
    /// cursor into the scrollback so the cursor line stays visible (lines
    /// below it are dropped first); growing brings lines back from the
    /// scrollback (primary screen). Columns are cut or padded, without
    /// reflowing. The cursor is clamped, the scroll region reset to the
    /// whole screen and the tab stops reset to every 8 columns.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = usize::from(cols.clamp(1, MAX_COLS));
        let rows = usize::from(rows.clamp(1, MAX_ROWS));
        if cols == self.cols && rows == self.rows {
            return;
        }
        for line in self.screen.iter_mut().chain(self.other.iter_mut()) {
            line.set_width(cols);
        }
        let alt = self.modes.alt_screen;
        let cap = self.scrollback_cap;
        // The primary screen's cursor: the live one, or the one saved on
        // entering the alternate screen.
        let mut cur_row = self.cur.row;
        let (primary, alternate) =
            if alt { (&mut self.other, &mut self.screen) } else { (&mut self.screen, &mut self.other) };
        let mut primary_row = if alt { self.saved[0].map_or(self.rows - 1, |s| s.cur.row) } else { cur_row };
        let moved = screen::resize_rows(primary, &mut primary_row, rows, cols, Some((&mut self.scrollback, cap)));
        let mut alt_row = if alt { cur_row } else { 0 };
        screen::resize_rows(alternate, &mut alt_row, rows, cols, None);
        if alt {
            cur_row = alt_row;
            if let Some(s) = self.saved[0].as_mut() {
                s.cur.row = primary_row;
            }
        } else {
            cur_row = primary_row;
        }
        if moved > 0 && cap > 0 {
            self.scrolled += moved as u64;
        }
        self.cols = cols;
        self.rows = rows;
        self.cur.row = cur_row;
        self.cur.col = self.cur.col.min(cols - 1);
        self.cur.pending_wrap = false;
        for s in self.saved.iter_mut().flatten() {
            s.cur.row = s.cur.row.min(rows - 1);
            s.cur.col = s.cur.col.min(cols - 1);
            s.cur.pending_wrap = false;
        }
        self.top = 0;
        self.bottom = rows - 1;
        self.tabs = default_tabs(cols);
        self.view_offset = self.view_offset.min(self.scrollback_len());
        self.dirty = vec![false; rows];
        self.damage_all = true;
    }

    fn reply(&mut self, s: &str) {
        self.replies.extend_from_slice(s.as_bytes());
    }

    /// Full reset (`ESC c`): screens, modes, pen, tabs, title kept;
    /// scrollback kept.
    fn full_reset(&mut self) {
        let blank = Line::blank(self.cols, Cell::BLANK);
        if self.modes.alt_screen {
            core::mem::swap(&mut self.screen, &mut self.other);
        }
        self.screen.fill(blank.clone());
        self.other.fill(blank);
        self.cur = CursorState::default();
        self.pen = Pen::default();
        self.charsets = [Charset::Ascii; 2];
        self.gl = 0;
        self.saved = [None; 2];
        self.top = 0;
        self.bottom = self.rows - 1;
        self.tabs = default_tabs(self.cols);
        self.modes = Modes::default();
        self.cursor_visible = true;
        self.cursor_shape = CursorShape::Block;
        self.cursor_blink = false;
        self.last_char = None;
        self.view_offset = 0;
        self.parser.reset();
        self.damage_all = true;
    }

    /// Soft reset (`CSI ! p`, DECSTR): modes and pen, not the screen.
    fn soft_reset(&mut self) {
        self.cursor_visible = true;
        self.modes.insert = false;
        self.modes.origin = false;
        self.modes.autowrap = true;
        self.modes.app_cursor = false;
        self.modes.app_keypad = false;
        self.top = 0;
        self.bottom = self.rows - 1;
        self.pen = Pen::default();
        self.charsets = [Charset::Ascii; 2];
        self.gl = 0;
        self.saved[usize::from(self.modes.alt_screen)] = None;
        self.cur.pending_wrap = false;
    }

    fn save_cursor(&mut self) {
        self.saved[usize::from(self.modes.alt_screen)] = Some(Saved {
            cur: self.cur,
            pen: self.pen,
            charsets: self.charsets,
            gl: self.gl,
            origin: self.modes.origin,
        });
    }

    fn restore_cursor(&mut self) {
        let s = self.saved[usize::from(self.modes.alt_screen)].unwrap_or_default();
        self.cur = s.cur;
        self.cur.row = self.cur.row.min(self.rows - 1);
        self.cur.col = self.cur.col.min(self.cols - 1);
        self.pen = s.pen;
        self.charsets = s.charsets;
        self.gl = s.gl;
        self.modes.origin = s.origin;
    }

    fn set_alt_screen(&mut self, on: bool, clear: bool) {
        if on == self.modes.alt_screen {
            if on && clear {
                self.clear_screen();
            }
            return;
        }
        if !on && clear {
            self.clear_screen();
        }
        core::mem::swap(&mut self.screen, &mut self.other);
        self.modes.alt_screen = on;
        if on && clear {
            self.clear_screen();
        }
        self.view_offset = 0;
        self.damage_all = true;
    }

    fn clear_screen(&mut self) {
        let blank = Line::blank(self.cols, Cell::blank(self.pen.bg));
        self.screen.fill(blank);
        self.damage_all = true;
    }

    fn osc_color_reply(&mut self, which: &str, rgb: (u8, u8, u8), bel: bool) {
        let (r, g, b) = rgb;
        let s = alloc::format!(
            "\x1b]{which};rgb:{:04x}/{:04x}/{:04x}{}",
            u16::from(r) * 0x101,
            u16::from(g) * 0x101,
            u16::from(b) * 0x101,
            if bel { "\x07" } else { "\x1b\\" }
        );
        self.reply(&s);
    }
}

impl Perform for Terminal {
    fn print(&mut self, c: char) {
        self.print_char(c);
    }

    fn execute(&mut self, c: char) {
        match c {
            '\x07' => self.bell = true,
            '\x08' => self.backspace(),
            '\t' => self.tab(1),
            '\n' | '\x0b' | '\x0c' => {
                self.linefeed();
                if self.modes.newline {
                    self.carriage_return();
                }
            }
            '\r' => self.carriage_return(),
            '\x0e' => self.gl = 1,
            '\x0f' => self.gl = 0,
            _ => {}
        }
    }

    fn esc(&mut self, intermediates: &[u8], f: u8) {
        match (intermediates, f) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.index(),
            ([], b'E') => {
                self.carriage_return();
                self.index();
            }
            ([], b'M') => self.reverse_index(),
            ([], b'H') => {
                if let Some(t) = self.tabs.get_mut(self.cur.col) {
                    *t = true;
                }
            }
            ([], b'c') => self.full_reset(),
            ([], b'=') => self.modes.app_keypad = true,
            ([], b'>') => self.modes.app_keypad = false,
            ([b'('], f) => self.charsets[0] = Charset::from_final(f),
            ([b')'], f) => self.charsets[1] = Charset::from_final(f),
            ([b'#'], b'8') => self.alignment_test(),
            _ => {}
        }
    }

    fn csi(&mut self, csi: &Csi) {
        self.dispatch_csi(csi);
    }

    fn osc(&mut self, data: &str, bel: bool) {
        let (cmd, arg) = data.split_once(';').unwrap_or((data, ""));
        match cmd {
            "0" | "2" => {
                self.title.clear();
                self.title.extend(arg.chars().filter(|c| !c.is_control()).take(MAX_TITLE));
                self.title_changed = true;
            }
            "7" => {
                self.cwd.clear();
                self.cwd.extend(arg.chars().filter(|c| !c.is_control()).take(MAX_TITLE));
            }
            "10" if arg == "?" => self.osc_color_reply("10", self.default_fg, bel),
            "11" if arg == "?" => self.osc_color_reply("11", self.default_bg, bel),
            _ => {}
        }
    }
}
