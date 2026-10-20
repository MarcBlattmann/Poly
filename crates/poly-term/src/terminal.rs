//! A text grid shared by a shell and the kernel log, with a status bar.

use crate::screen::Screen;
use crate::status::{self, Line, StatusInfo};
use core::fmt::Write;
use poly_gfx::{Attrs, Canvas, Cell, GridError, Paint, Palette, Rect, Style, TextGrid};

/// How kernel log text looks on the screen: grey, so the shell's own
/// output stands out.
pub const LOG_STYLE: Style = Style { fg: Paint::Rgb(0x8a, 0x90, 0x9c), bg: Paint::Default, attrs: Attrs::empty() };

/// Whether the shell's prompt and line are on the cursor's row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prompt {
    /// No prompt (a command is running, or no shell).
    None,
    /// The shell waits for input with its prompt and line on the current row.
    Shown,
    /// Log output erased that row; the shell has to draw it again.
    Erased,
}

/// The screen terminal: a status bar and a [`TextGrid`] laid out on a
/// [`Screen`], with one shell and the kernel log writing into the grid.
///
/// The log never lands in the middle of the line being typed. While the
/// shell waits for input it says so ([`prompt_shown`](Self::prompt_shown));
/// the next log output first erases the prompt row and marks the prompt
/// erased, and the shell, which asks [`take_redraw`](Self::take_redraw)
/// while it waits, draws its prompt and line again below the log (the
/// line editor's `Edit::Redraw`). That is what the serial console does
/// with the same editor.
#[derive(Debug)]
pub struct Terminal<'a> {
    screen: Screen,
    grid: TextGrid<'a>,
    palette: Palette,
    prompt: Prompt,
    /// The last log output did not end with a newline.
    log_mid_line: bool,
    info: Option<StatusInfo>,
    /// What the status bar shows now.
    drawn_status: Line,
    /// Nothing has been drawn yet (or it was invalidated): paint it all.
    full: bool,
}

impl<'a> Terminal<'a> {
    /// A blank terminal for `screen` in `cells`, which must hold
    /// [`Screen::cells`] cells (more become extra scrollback).
    pub fn new(screen: Screen, cells: &'a mut [Cell]) -> Result<Terminal<'a>, GridError> {
        let grid = TextGrid::new(cells, screen.cols, screen.rows)?;
        Ok(Terminal {
            screen,
            grid,
            palette: Palette::DEFAULT,
            prompt: Prompt::None,
            log_mid_line: false,
            info: None,
            drawn_status: Line::default(),
            full: true,
        })
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn grid(&self) -> &TextGrid<'a> {
        &self.grid
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Text and escape sequences from the shell.
    pub fn write_shell(&mut self, s: &str) {
        self.grid.write_str(s);
        self.log_mid_line = false;
    }

    /// Bytes of kernel log output (UTF-8, possibly cut in the middle of a
    /// character; the rest comes with the next call). If the prompt is
    /// shown, its row is erased first and the prompt marked erased.
    pub fn write_log(&mut self, bytes: &[u8]) {
        let Some(&last) = bytes.last() else { return };
        if self.prompt == Prompt::Shown {
            self.grid.write_str("\r\x1b[K");
            self.prompt = Prompt::Erased;
        }
        let shell_style = self.grid.style();
        self.grid.set_style(LOG_STYLE);
        self.grid.write_bytes(bytes);
        self.grid.set_style(shell_style);
        self.log_mid_line = last != b'\n';
    }

    /// Note that `bytes` of log output were lost (the log ring overran
    /// before they were shown), on a line of its own.
    pub fn log_lost(&mut self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let mut note = Line::default();
        let newline = if self.log_mid_line { "\n" } else { "" };
        let _ = writeln!(note, "{newline}[{bytes} bytes of kernel log lost]");
        self.write_log(note.as_str().as_bytes());
    }

    /// The shell's prompt and line are on the current row (true), or not
    /// any more (false: the line was submitted or cancelled). Marking it
    /// shown keeps a pending "erased", so a redraw is not lost.
    pub fn prompt_shown(&mut self, shown: bool) {
        match (shown, self.prompt) {
            (true, Prompt::None) => self.prompt = Prompt::Shown,
            (true, _) => {}
            (false, _) => self.prompt = Prompt::None,
        }
    }

    /// Whether log output erased the prompt since it was drawn; if so the
    /// caller draws it again, so it counts as shown from here. The cursor
    /// is then at the start of a row: after a log line that did not end in
    /// a newline, one is added.
    pub fn take_redraw(&mut self) -> bool {
        if self.prompt != Prompt::Erased {
            return false;
        }
        self.prompt = Prompt::Shown;
        if self.log_mid_line {
            self.grid.write_str("\n");
            self.log_mid_line = false;
        }
        true
    }

    /// Whether [`take_redraw`](Self::take_redraw) would return true.
    pub fn redraw_pending(&self) -> bool {
        self.prompt == Prompt::Erased
    }

    /// Scroll the view `pages` screens back into the scrollback (negative:
    /// toward the screen); a page is the screen's rows less one, so a line
    /// stays in view for orientation.
    pub fn scroll_pages(&mut self, pages: i32) {
        let page = self.screen.rows.saturating_sub(1).max(1) as isize;
        self.grid.scroll_view((pages as isize).saturating_mul(page));
    }

    /// Show the screen again, not the scrollback.
    pub fn scroll_to_bottom(&mut self) {
        self.grid.scroll_view_to_bottom();
    }

    /// What the status bar shows from the next render on.
    pub fn set_status(&mut self, info: StatusInfo) {
        self.info = Some(info);
    }

    /// The status line as it would be drawn now: the facts last set, and
    /// where the view is. Blank before [`set_status`](Self::set_status).
    pub fn status_line(&self) -> Line {
        match self.info {
            Some(info) => {
                let scrolled = Some((self.grid.view_offset(), self.grid.history()));
                status::format(&StatusInfo { scrolled, ..info }, self.screen.cols)
            }
            None => Line::default(),
        }
    }

    /// Whether [`render`](Self::render) would draw anything.
    pub fn needs_render(&self) -> bool {
        self.full || self.grid.needs_render() || self.status_line() != self.drawn_status
    }

    /// Forget what the canvas shows: the next render paints everything.
    pub fn invalidate(&mut self) {
        self.full = true;
    }

    /// Draw what changed onto `canvas` (the whole display): the status bar
    /// if its text changed, the grid's changed cells. The first render (and
    /// the first after [`invalidate`](Self::invalidate)) paints the whole
    /// display. Returns the rectangle drawn on, for flushing.
    pub fn render(&mut self, canvas: &mut Canvas<'_>) -> Option<Rect> {
        let mut changed = Rect::default();
        let full = core::mem::replace(&mut self.full, false);
        if full {
            canvas.fill_rect(self.screen.bounds(), self.palette.background);
            self.grid.invalidate();
            changed = self.screen.bounds().intersect(&canvas.bounds());
        }
        let line = self.status_line();
        if full || line != self.drawn_status {
            let r = status::draw(canvas, self.screen.status, self.screen.font, line.as_str());
            changed = changed.union(&r);
            self.drawn_status = line;
        }
        let (x, y) = (self.screen.grid.x, self.screen.grid.y);
        if let Some(r) = self.grid.render(canvas, x, y, self.screen.font, &self.palette) {
            changed = changed.union(&r);
        }
        (!changed.is_empty()).then_some(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_gfx::{Color, PixelFormat};
    use std::string::String;
    use std::vec;
    use std::vec::Vec;

    const PROMPT: &str = "\x1b[1;32mpoly>\x1b[0m ";

    fn screen() -> Screen {
        // 40 x 5 cells and a status row.
        Screen::new(320, 96).unwrap()
    }

    fn row(t: &Terminal<'_>, r: usize) -> String {
        let s: String = t.grid().screen_line(r).unwrap().iter().map(|c| c.ch).collect();
        String::from(s.trim_end())
    }

    fn info() -> StatusInfo {
        StatusInfo {
            version: "0.0.1",
            uptime_ns: 5_000_000_000,
            free_bytes: 200 << 20,
            total_bytes: 256 << 20,
            layout: "us",
            cpus_online: 1,
            cpus: 2,
            scrolled: None,
        }
    }

    #[test]
    fn log_output_erases_the_prompt_and_asks_for_a_redraw() {
        let s = screen();
        let mut cells = vec![Cell::BLANK; s.cells()];
        let mut t = Terminal::new(s, &mut cells).unwrap();
        assert_eq!((t.screen().cols, t.screen().rows), (40, 5));
        t.write_shell(PROMPT);
        t.write_shell("ec");
        t.prompt_shown(true);
        assert!(!t.take_redraw(), "nothing disturbed it yet");
        t.write_log(b"disk: ready\n");
        t.write_log(b"net: up\n");
        assert!(t.redraw_pending());
        assert_eq!((row(&t, 0), row(&t, 1), row(&t, 2)), ("disk: ready".into(), "net: up".into(), "".into()));
        assert_eq!(t.grid().cell(0, 0).unwrap().style, LOG_STYLE);
        assert!(t.take_redraw());
        assert!(!t.take_redraw(), "once");
        // What the editor's redraw writes.
        t.write_shell("\r\x1b[K");
        t.write_shell(PROMPT);
        t.write_shell("ec");
        t.write_shell("ho");
        assert_eq!(row(&t, 2), "poly> echo");
        assert_eq!(t.grid().cell(6, 2).unwrap().style, Style::default(), "the shell's text keeps its own style");
        // Enter: the line is done; log output goes anywhere again.
        t.write_shell("\n");
        t.prompt_shown(false);
        t.write_log(b"late line\n");
        assert!(!t.redraw_pending());
        assert_eq!(row(&t, 3), "late line");
    }

    #[test]
    fn a_log_line_without_newline_is_ended_before_the_redraw() {
        let s = screen();
        let mut cells = vec![Cell::BLANK; s.cells()];
        let mut t = Terminal::new(s, &mut cells).unwrap();
        t.write_shell(PROMPT);
        t.prompt_shown(true);
        t.write_log(b"partial");
        assert!(t.take_redraw());
        assert_eq!(t.grid().cursor(), (0, 1));
        assert_eq!(row(&t, 0), "partial");
        // Shell output after log output (the echo of a key typed just then)
        // stays on the log's row; the redraw erases that row.
        t.write_shell("poly> x");
        t.write_log(b"more\n");
        t.write_shell("y");
        assert!(t.take_redraw());
        assert_eq!(t.grid().cursor(), (1, 2), "no newline added after shell output");
    }

    #[test]
    fn marking_shown_keeps_a_pending_redraw() {
        let s = screen();
        let mut cells = vec![Cell::BLANK; s.cells()];
        let mut t = Terminal::new(s, &mut cells).unwrap();
        t.prompt_shown(true);
        t.write_log(b"x\n");
        t.prompt_shown(true);
        assert!(t.take_redraw());
        t.prompt_shown(false);
        t.write_log(b"");
        t.log_lost(0);
        assert!(!t.redraw_pending());
        t.write_log(b"abc");
        t.log_lost(1234);
        assert_eq!((row(&t, 1), row(&t, 2)), ("abc".into(), "[1234 bytes of kernel log lost]".into()));
    }

    #[test]
    fn pages_scroll_the_view_and_the_status_says_so() {
        // Wide enough for the whole status line.
        let s = Screen::new(1024, 96).unwrap();
        let mut cells = vec![Cell::BLANK; s.cells()];
        let mut t = Terminal::new(s, &mut cells).unwrap();
        t.set_status(info());
        for i in 0..30 {
            t.write_shell(&std::format!("line {i}\n"));
        }
        assert!(t.status_line().as_str().ends_with("Shift+PgUp: scrollback "));
        t.scroll_pages(1);
        assert_eq!(t.grid().view_offset(), 4);
        assert!(t.status_line().as_str().contains("scrollback 4 of 26 lines"), "{:?}", t.status_line());
        t.scroll_pages(100);
        assert_eq!(t.grid().view_offset(), 26);
        t.scroll_pages(-1);
        assert_eq!(t.grid().view_offset(), 22);
        t.scroll_to_bottom();
        assert_eq!(t.grid().view_offset(), 0);
    }

    /// A canvas the size of the screen.
    fn canvas_bytes(s: &Screen) -> Vec<u8> {
        vec![0u8; (s.width * s.height * 4) as usize]
    }

    #[test]
    fn renders_status_bar_and_grid_and_then_only_changes() {
        let s = screen();
        let mut cells = vec![Cell::BLANK; s.cells()];
        let mut t = Terminal::new(s, &mut cells).unwrap();
        let mut buf = canvas_bytes(&s);
        let mut c = Canvas::new(&mut buf, s.width, s.height, s.width as usize * 4, PixelFormat::Xrgb8888).unwrap();
        t.set_status(info());
        t.write_shell("hi");
        assert!(t.needs_render());
        assert_eq!(t.render(&mut c), Some(s.bounds()));
        assert!(!t.needs_render());
        assert_eq!(t.render(&mut c), None);
        // The status bar: only its two colours, and some text.
        let px = |x: i32, y: i32| c.pixel(x, y).unwrap();
        let bar: Vec<Color> = (0..16).flat_map(|y| (0..320).map(move |x| (x, y))).map(|(x, y)| px(x, y)).collect();
        assert!(bar.iter().all(|&p| p == status::BACKGROUND || p == status::FOREGROUND));
        assert!(bar.iter().filter(|&&p| p == status::FOREGROUND).count() > 100);
        // The grid starts below it: "hi" in the default colours, then the
        // cursor (a block) in the third cell.
        let p = Palette::DEFAULT;
        let cell = |col: i32| (16..32).flat_map(move |y| (col * 8..col * 8 + 8).map(move |x| (x, y)));
        assert!(cell(0).any(|(x, y)| px(x, y) == p.foreground));
        assert!(cell(2).all(|(x, y)| px(x, y) == p.cursor));
        assert!(cell(3).all(|(x, y)| px(x, y) == p.background));
        // Only the status line changed: only the bar is drawn.
        t.set_status(StatusInfo { uptime_ns: 6_000_000_000, ..info() });
        assert_eq!(t.render(&mut c), Some(s.status));
        // A character: its cell and the cursor's.
        t.write_shell("!");
        assert_eq!(t.render(&mut c), Some(Rect::new(16, 16, 16, 16)));
        t.invalidate();
        assert_eq!(t.render(&mut c), Some(s.bounds()));
    }

    #[test]
    fn the_margin_is_background() {
        // 7 pixels right of the last column, 10 below the last row.
        let s = Screen::new(327, 106).unwrap();
        let mut cells = vec![Cell::BLANK; s.cells()];
        let mut t = Terminal::new(s, &mut cells).unwrap();
        let mut buf = vec![0xffu8; (s.width * s.height * 4) as usize];
        let mut c = Canvas::new(&mut buf, s.width, s.height, s.width as usize * 4, PixelFormat::Xrgb8888).unwrap();
        t.render(&mut c);
        assert_eq!(c.pixel(326, 105), Some(Palette::DEFAULT.background));
        assert_eq!(c.pixel(326, 0), Some(status::BACKGROUND), "the bar spans the full width");
    }
}
