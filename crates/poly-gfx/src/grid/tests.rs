extern crate std;

use super::*;
use crate::canvas::Canvas;
use crate::color::{Color, PixelFormat};
use crate::font::{SPLEEN_8X16, SPLEEN_12X24};
use crate::geom::Rect;
use std::format;
use std::string::String;
use std::vec;
use std::vec::Vec;

fn storage(cols: usize, rows: usize, scrollback: usize) -> Vec<Cell> {
    vec![Cell::BLANK; TextGrid::storage_len(cols, rows, scrollback)]
}

/// The screen as text, trailing blanks removed.
fn screen(g: &TextGrid<'_>) -> Vec<String> {
    (0..g.rows()).map(|r| line_text(g.screen_line(r).unwrap())).collect()
}

fn line_text(cells: &[Cell]) -> String {
    cells.iter().map(|c| c.ch).collect::<String>().trim_end().into()
}

/// The scrollback as text, oldest first.
fn history(g: &TextGrid<'_>) -> Vec<String> {
    (0..g.history()).rev().map(|n| line_text(g.history_line(n).unwrap())).collect()
}

#[test]
fn new_checks_its_arguments() {
    let mut cells = storage(10, 3, 0);
    assert_eq!(TextGrid::new(&mut cells, 0, 3).unwrap_err(), GridError::Empty);
    assert_eq!(TextGrid::new(&mut cells, 10, 0).unwrap_err(), GridError::Empty);
    assert_eq!(TextGrid::new(&mut cells, 10, 4).unwrap_err(), GridError::StorageTooSmall);
    assert_eq!(TextGrid::new(&mut cells, 1, MAX_ROWS + 1).unwrap_err(), GridError::TooLarge);
    assert_eq!(TextGrid::new(&mut cells, MAX_COLS + 1, 1).unwrap_err(), GridError::TooLarge);
    let g = TextGrid::new(&mut cells, 10, 3).unwrap();
    assert_eq!((g.cols(), g.rows(), g.scrollback_capacity(), g.history()), (10, 3, 0, 0));
    // Leftover cells that do not make a whole line are not used.
    let mut cells = vec![Cell::BLANK; 10 * 5 + 7];
    let g = TextGrid::new(&mut cells, 10, 3).unwrap();
    assert_eq!(g.scrollback_capacity(), 2);
}

#[test]
fn text_newlines_and_controls() {
    let mut cells = storage(10, 4, 0);
    let mut g = TextGrid::new(&mut cells, 10, 4).unwrap();
    g.write_str("hello\nwörld\r W\tx\x08y");
    assert_eq!(screen(&g), ["hello", " Wrld   y", "", ""]);
    assert_eq!(g.cursor(), (9, 1));
    // Tab stops every 8 columns, never past the last column.
    g.write_str("\r\t\t\tz");
    assert_eq!(g.cell(9, 1).unwrap().ch, 'z');
    // Without newline mode, \n only moves down.
    g.set_newline_mode(false);
    g.write_str("\rab\ncd");
    assert_eq!(screen(&g)[2], "  cd");
    // Other controls and C1 controls are ignored.
    g.write_str("\x07\x00\u{85}e");
    assert_eq!(screen(&g)[2], "  cde");
}

#[test]
fn wraps_with_a_pending_wrap() {
    let mut cells = storage(5, 3, 0);
    let mut g = TextGrid::new(&mut cells, 5, 3).unwrap();
    g.write_str("abcde");
    // At the last column the cursor waits; a newline does not add a line.
    assert_eq!(g.cursor(), (4, 0));
    g.write_str("\nfghijk");
    assert_eq!(screen(&g), ["abcde", "fghij", "k"]);
    // A carriage return cancels the pending wrap.
    let mut cells = storage(5, 3, 0);
    let mut g = TextGrid::new(&mut cells, 5, 3).unwrap();
    g.write_str("abcde\rX");
    assert_eq!(screen(&g), ["Xbcde", "", ""]);
    // So does backspace, which then moves left.
    g.write_str("\rabcde\x08Y");
    assert_eq!(screen(&g), ["abcYe", "", ""]);
    // Autowrap off: the last column is overwritten.
    g.write_str("\x1b[?7l\r12345678\x1b[?7h");
    assert_eq!(screen(&g), ["12348", "", ""]);
}

#[test]
fn scrolls_into_a_bounded_scrollback() {
    let mut cells = storage(4, 2, 3);
    let mut g = TextGrid::new(&mut cells, 4, 2).unwrap();
    for i in 0..4 {
        g.write_str(&format!("l{i}\n"));
    }
    assert_eq!(screen(&g), ["l3", ""]);
    assert_eq!(history(&g), ["l0", "l1", "l2"]);
    g.write_str("l4\nl5\n");
    assert_eq!(history(&g), ["l2", "l3", "l4"]);
    assert_eq!(screen(&g), ["l5", ""]);
    assert_eq!(g.history_line(0).map(line_text), Some("l4".into()));
    assert_eq!(g.history_line(3), None);
    // ED 3 empties the scrollback but keeps the screen.
    g.write_str("x\x1b[3J");
    assert_eq!((history(&g).len(), screen(&g)), (0, vec![String::from("l5"), "x".into()]));
    g.write_str("\ny");
    assert_eq!(history(&g), ["l5"]);
    assert_eq!(screen(&g), ["x", "y"]);
}

#[test]
fn cursor_movement_clamps() {
    let mut cells = storage(10, 5, 0);
    let mut g = TextGrid::new(&mut cells, 10, 5).unwrap();
    let moves = [
        ("\x1b[3;4H", (3, 2)),
        ("\x1b[H", (0, 0)),
        ("\x1b[99;99H", (9, 4)),
        ("\x1b[2A", (9, 2)),
        ("\x1b[A", (9, 1)),
        ("\x1b[0A", (9, 0)),
        ("\x1b[5D", (4, 0)),
        ("\x1b[3B", (4, 3)),
        ("\x1b[9C", (9, 3)),
        ("\x1b[2F", (0, 1)),
        ("\x1b[E", (0, 2)),
        ("\x1b[7G", (6, 2)),
        ("\x1b[1d", (6, 0)),
        ("\x1b[;3f", (2, 0)),
        ("\x1b[0;0H", (0, 0)),
    ];
    for (seq, want) in moves {
        g.write_str(seq);
        assert_eq!(g.cursor(), want, "{seq:?}");
    }
    // Save and restore, with the style.
    g.write_str("\x1b[2;3H\x1b[31m\x1b7\x1b[0m\x1b[5;5H\x1b8");
    assert_eq!((g.cursor(), g.style().fg), ((2, 1), Paint::Indexed(1)));
    g.write_str("\x1b[0m\x1b[4;1H\x1b[s\x1b[H\x1b[u");
    assert_eq!(g.cursor(), (0, 3));
}

#[test]
fn erasing() {
    let mut cells = storage(6, 3, 0);
    let mut g = TextGrid::new(&mut cells, 6, 3).unwrap();
    let fill = "\x1b[Habcdef\x1b[2;1Hghijkl\x1b[3;1Hmnopqr\x1b[2;3H";
    let cases = [
        ("\x1b[K", ["abcdef", "gh", "mnopqr"]),
        ("\x1b[1K", ["abcdef", "   jkl", "mnopqr"]),
        ("\x1b[2K", ["abcdef", "", "mnopqr"]),
        ("\x1b[J", ["abcdef", "gh", ""]),
        ("\x1b[1J", ["", "   jkl", "mnopqr"]),
        ("\x1b[2J", ["", "", ""]),
        ("\x1b[2X", ["abcdef", "gh  kl", "mnopqr"]),
        ("\x1b[2@", ["abcdef", "gh  ij", "mnopqr"]),
        ("\x1b[9@", ["abcdef", "gh", "mnopqr"]),
        ("\x1b[2P", ["abcdef", "ghkl", "mnopqr"]),
        ("\x1b[L", ["abcdef", "", "ghijkl"]),
        ("\x1b[M", ["abcdef", "mnopqr", ""]),
        ("\x1b[5M", ["abcdef", "", ""]),
        ("\x1b[T", ["", "abcdef", "ghijkl"]),
        ("\x1b[S", ["ghijkl", "mnopqr", ""]),
    ];
    for (seq, want) in cases {
        g.write_str(fill);
        g.write_str(seq);
        assert_eq!(screen(&g), want, "{seq:?}");
    }
    // Erasing does not move the cursor; the scrolls keep it too.
    assert_eq!(g.cursor(), (2, 1));
}

#[test]
fn erased_cells_take_the_background_colour() {
    let mut cells = storage(4, 2, 0);
    let mut g = TextGrid::new(&mut cells, 4, 2).unwrap();
    g.write_str("ab\x1b[1;44m\x1b[K\n");
    assert_eq!(g.cell(0, 0).unwrap().style, Style::default());
    let blank = Cell { ch: ' ', style: Style { fg: Paint::Default, bg: Paint::Indexed(4), attrs: Attrs::empty() } };
    assert_eq!(g.cell(2, 0), Some(blank));
    // A line scrolled in gets the background too.
    g.write_str("\n");
    assert_eq!(g.cell(0, 1), Some(blank));
}

#[test]
fn styles() {
    let mut cells = storage(20, 1, 0);
    let mut g = TextGrid::new(&mut cells, 20, 1).unwrap();
    let style = |g: &mut TextGrid<'_>, seq: &str| {
        g.write_str(seq);
        g.style()
    };
    let s = style(&mut g, "\x1b[1;4;31;42m");
    assert_eq!(s.fg, Paint::Indexed(1));
    assert_eq!(s.bg, Paint::Indexed(2));
    assert!(s.attrs.contains(Attrs::BOLD.union(Attrs::UNDERLINE)));
    let s = style(&mut g, "\x1b[22;24;93;104m");
    assert_eq!((s.fg, s.bg, s.attrs), (Paint::Indexed(11), Paint::Indexed(12), Attrs::empty()));
    let s = style(&mut g, "\x1b[38;5;200;48;2;1;2;3m");
    assert_eq!((s.fg, s.bg), (Paint::Indexed(200), Paint::Rgb(1, 2, 3)));
    let s = style(&mut g, "\x1b[2;7;8;9m");
    assert_eq!(s.attrs, Attrs::FAINT.union(Attrs::INVERSE).union(Attrs::HIDDEN).union(Attrs::STRIKE));
    let s = style(&mut g, "\x1b[27;28;29;39;49m");
    assert_eq!(s, Style { attrs: Attrs::FAINT, ..Style::default() });
    assert_eq!(style(&mut g, "\x1b[m"), Style::default());
    // Truncated extended colours change nothing, and the rest of the
    // sequence is dropped.
    assert_eq!(style(&mut g, "\x1b[38;5m\x1b[48;2;1;2m\x1b[38;9;1m"), Style::default());
    assert_eq!(style(&mut g, "\x1b[38;2;999;0;0m").fg, Paint::Rgb(255, 0, 0));
    // The style goes into the cells written.
    g.write_str("\x1b[0;35mX");
    assert_eq!(g.cell(0, 0).unwrap().style.fg, Paint::Indexed(5));
}

#[test]
fn modes_and_resets() {
    let mut cells = storage(5, 3, 2);
    let mut g = TextGrid::new(&mut cells, 5, 3).unwrap();
    g.write_str("\x1b[?25l");
    assert!(!g.cursor_visible());
    g.write_str("\x1b[?25h");
    assert!(g.cursor_visible());
    // Reverse index at the top inserts a line; index at the bottom scrolls.
    g.write_str("a\x1bM");
    assert_eq!(screen(&g), ["", "a", ""]);
    g.write_str("\x1b[3;1Hb\x1bD");
    assert_eq!(screen(&g), ["a", "b", ""]);
    assert_eq!(g.cursor(), (1, 2));
    g.write_str("\x1bEc");
    assert_eq!(screen(&g), ["b", "", "c"]);
    // Unknown sequences and private sequences do nothing.
    g.write_str("\x1b[5n\x1b[>c\x1b[=1u\x1b]2;title\x07\x1b(0");
    assert_eq!(screen(&g), ["b", "", "c"]);
    g.write_str("\x1b[31m\x1b[?25l\x1bc");
    assert_eq!(screen(&g), ["", "", ""]);
    assert_eq!((g.history(), g.cursor(), g.style(), g.cursor_visible()), (0, (0, 0), Style::default(), true));
}

#[test]
fn utf8_bytes_may_be_split() {
    let mut cells = storage(10, 1, 0);
    let mut g = TextGrid::new(&mut cells, 10, 1).unwrap();
    let bytes = "Grüezi€".as_bytes();
    for b in bytes {
        g.write_bytes(core::slice::from_ref(b));
    }
    g.write_bytes(b"\xff");
    assert_eq!(screen(&g), ["Grüezi€\u{fffd}"]);
    // An unfinished character before text is replaced.
    g.write_bytes(b"\xc3");
    g.write_str("!");
    assert_eq!(screen(&g), ["Grüezi€\u{fffd}\u{fffd}!"]);
    // The line is full; the next character wraps, scrolling the one row.
    assert_eq!(g.cursor(), (9, 0));
    g.write_char('x');
    assert_eq!((screen(&g), g.cursor()), (vec![String::from("x")], (1, 0)));
}

#[test]
fn view_scrolls_back_and_stays_put() {
    let mut cells = storage(4, 2, 4);
    let mut g = TextGrid::new(&mut cells, 4, 2).unwrap();
    g.write_str("0\n1\n2\n3\n4");
    assert_eq!(history(&g), ["0", "1", "2"]);
    g.scroll_view(2);
    assert_eq!(g.view_offset(), 2);
    let shown = |g: &TextGrid<'_>| -> Vec<String> { (0..2).map(|r| line_text(g.line(g.visible_ring(r)))).collect() };
    assert_eq!(shown(&g), ["1", "2"]);
    // New output does not move what is shown.
    g.write_str("\n5");
    assert_eq!((g.view_offset(), shown(&g)), (3, vec![String::from("1"), "2".into()]));
    // The scrollback is full: the view holds on to its lines until they
    // drop out.
    g.write_str("\n6");
    assert_eq!(history(&g), ["1", "2", "3", "4"]);
    assert_eq!((g.view_offset(), shown(&g)), (4, vec![String::from("1"), "2".into()]));
    g.write_str("\n7");
    assert_eq!(history(&g), ["2", "3", "4", "5"]);
    assert_eq!((g.view_offset(), shown(&g)), (4, vec![String::from("2"), "3".into()]));
    g.scroll_view(100);
    assert_eq!(g.view_offset(), 4);
    g.scroll_view(-1);
    assert_eq!(shown(&g), ["3", "4"]);
    g.scroll_view_to_bottom();
    assert_eq!((g.view_offset(), shown(&g)), (0, vec![String::from("6"), "7".into()]));
}

// --- Rendering ------------------------------------------------------------------

const FONT: &crate::Font = &SPLEEN_8X16;
const MARGIN: i32 = 5;
const OUTSIDE: Color = Color::from_rgb24(0x123456);

/// A canvas big enough for a `cols` × `rows` grid at (MARGIN, MARGIN) with a
/// margin all around, filled with OUTSIDE.
fn canvas_buf(cols: usize, rows: usize, font: &crate::Font) -> (Vec<u8>, u32, u32) {
    let w = cols as u32 * font.width() + 2 * MARGIN as u32;
    let h = rows as u32 * font.height() + 2 * MARGIN as u32;
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let mut c = Canvas::new(&mut buf, w, h, w as usize * 4, PixelFormat::Xrgb8888).unwrap();
    c.clear(OUTSIDE);
    (buf, w, h)
}

fn canvas(buf: &mut [u8], w: u32, h: u32) -> Canvas<'_> {
    Canvas::new(buf, w, h, w as usize * 4, PixelFormat::Xrgb8888).unwrap()
}

fn cell_rect(col: usize, row: usize) -> Rect {
    Rect::new(MARGIN + 8 * col as i32, MARGIN + 16 * row as i32, 8, 16)
}

#[test]
fn render_draws_everything_once_then_only_changes() {
    let (cols, rows) = (6, 3);
    let mut cells = storage(cols, rows, 0);
    let mut g = TextGrid::new(&mut cells, cols, rows).unwrap();
    let (mut buf, w, h) = canvas_buf(cols, rows, FONT);
    let p = Palette::DEFAULT;
    let mut c = canvas(&mut buf, w, h);
    assert!(g.needs_render());
    let all = g.render(&mut c, MARGIN, MARGIN, FONT, &p);
    assert_eq!(all, Some(Rect::new(MARGIN, MARGIN, 48, 48)));
    assert!(!g.needs_render());
    assert_eq!(g.render(&mut c, MARGIN, MARGIN, FONT, &p), None);
    // The margin is untouched, the grid is background with a cursor block.
    assert_eq!(c.pixel(MARGIN - 1, MARGIN), Some(OUTSIDE));
    assert_eq!(c.pixel(MARGIN + 48, MARGIN), Some(OUTSIDE));
    assert_eq!(c.pixel(MARGIN + 8, MARGIN), Some(p.background));
    assert_eq!(c.pixel(MARGIN, MARGIN), Some(p.cursor));

    // One character: its cell and the cell the cursor moved to.
    g.write_str("\x1b[2;3HX");
    let r = g.render(&mut c, MARGIN, MARGIN, FONT, &p).unwrap();
    // The old cursor (0,0), "X" at (2,1) and the new cursor at (3,1).
    assert_eq!(r, cell_rect(0, 0).union(&cell_rect(3, 1)));
    assert_eq!(c.pixel(MARGIN, MARGIN), Some(p.background));
    assert_eq!(c.pixel(cell_rect(3, 1).x, cell_rect(3, 1).y), Some(p.cursor));

    // Hiding the cursor redraws just its cell.
    g.set_cursor_visible(false);
    assert_eq!(g.render(&mut c, MARGIN, MARGIN, FONT, &p), Some(cell_rect(3, 1)));
    // Writing in one row touches only that row's changed span.
    g.write_str("\x1b[3;2Hab");
    assert_eq!(g.render(&mut c, MARGIN, MARGIN, FONT, &p), Some(Rect::new(MARGIN + 8, MARGIN + 32, 16, 16)));
}

#[test]
fn render_leaves_unchanged_cells_alone() {
    let (cols, rows) = (8, 4);
    let mut cells = storage(cols, rows, 0);
    let mut g = TextGrid::new(&mut cells, cols, rows).unwrap();
    let (mut buf, w, h) = canvas_buf(cols, rows, FONT);
    let p = Palette::DEFAULT;
    let mut c = canvas(&mut buf, w, h);
    g.write_str("abc\ndef");
    g.render(&mut c, MARGIN, MARGIN, FONT, &p);
    // Paint over a cell the next change does not involve...
    c.fill_rect(cell_rect(1, 0), OUTSIDE);
    g.write_str("\x1b[3;1Hxyz\x1b[2Kq");
    g.render(&mut c, MARGIN, MARGIN, FONT, &p);
    // ...and it is still painted over.
    assert_eq!(c.pixel(cell_rect(1, 0).x + 3, cell_rect(1, 0).y + 9), Some(OUTSIDE));
    // After invalidate, it is redrawn.
    g.invalidate();
    g.render(&mut c, MARGIN, MARGIN, FONT, &p);
    assert_ne!(c.pixel(cell_rect(1, 0).x + 3, cell_rect(1, 0).y + 9), Some(OUTSIDE));
}

#[test]
fn render_uses_the_palette_and_attributes() {
    let mut cells = storage(4, 1, 0);
    let mut g = TextGrid::new(&mut cells, 4, 1).unwrap();
    let (mut buf, w, h) = canvas_buf(4, 1, FONT);
    let p = Palette::DEFAULT;
    let mut c = canvas(&mut buf, w, h);
    g.write_str("\x1b[31;47m█\x1b[7m█\x1b[0;8m█\x1b[0;38;2;1;2;3;4m_");
    g.set_cursor_visible(false);
    g.render(&mut c, MARGIN, MARGIN, FONT, &p);
    let at = |col: usize, x: i32, y: i32| c.pixel(cell_rect(col, 0).x + x, cell_rect(col, 0).y + y);
    // A full block shows the foreground everywhere it is lit.
    assert_eq!(at(0, 4, 8), Some(p.ansi[1]));
    assert_eq!(at(1, 4, 8), Some(p.ansi[7]));
    assert_eq!(at(2, 4, 8), Some(p.background)); // hidden
    // Underlined: the row below the baseline is lit across, in the RGB colour.
    assert_eq!(at(3, 0, FONT.ascent() as i32), Some(Color::rgb(1, 2, 3)));
    assert_eq!(p.indexed(16), Color::BLACK);
    assert_eq!(p.indexed(231), Color::WHITE);
    assert_eq!(p.indexed(196), Color::rgb(255, 0, 0));
    assert_eq!(p.indexed(232), Color::rgb(8, 8, 8));
    assert_eq!(p.indexed(255), Color::rgb(238, 238, 238));
}

#[test]
fn cursor_shapes() {
    let mut cells = storage(3, 1, 0);
    let mut g = TextGrid::new(&mut cells, 3, 1).unwrap();
    let (mut buf, w, h) = canvas_buf(3, 1, FONT);
    let p = Palette::DEFAULT;
    let mut c = canvas(&mut buf, w, h);
    g.render(&mut c, MARGIN, MARGIN, FONT, &p);
    g.set_cursor_shape(CursorShape::Underline);
    assert_eq!(g.render(&mut c, MARGIN, MARGIN, FONT, &p), Some(cell_rect(0, 0)));
    assert_eq!(c.pixel(MARGIN, MARGIN), Some(p.background));
    assert_eq!(c.pixel(MARGIN + 7, MARGIN + 15), Some(p.cursor));
    g.set_cursor_shape(CursorShape::Bar);
    g.render(&mut c, MARGIN, MARGIN, FONT, &p);
    assert_eq!(c.pixel(MARGIN + 7, MARGIN + 15), Some(p.background));
    assert_eq!(c.pixel(MARGIN + 1, MARGIN), Some(p.cursor));
    assert_eq!(g.cursor_shape(), CursorShape::Bar);
}

/// Scrolling moves pixels instead of redrawing: a cell that only moved is
/// not drawn again.
#[test]
fn scrolling_moves_pixels() {
    let (cols, rows) = (4, 3);
    let mut cells = storage(cols, rows, 10);
    let mut g = TextGrid::new(&mut cells, cols, rows).unwrap();
    let (mut buf, w, h) = canvas_buf(cols, rows, FONT);
    let p = Palette::DEFAULT;
    let mut c = canvas(&mut buf, w, h);
    g.write_str("a\nb\nc");
    g.render(&mut c, MARGIN, MARGIN, FONT, &p);
    // Mark the "c" cell's pixels; after one scroll they must show up a row
    // higher, moved rather than redrawn.
    c.set_pixel(MARGIN + 7, MARGIN + 32, OUTSIDE);
    g.write_str("\nd");
    let r = g.render(&mut c, MARGIN, MARGIN, FONT, &p);
    assert_eq!(r, Some(Rect::new(MARGIN, MARGIN, 32, 48)));
    assert_eq!(c.pixel(MARGIN + 7, MARGIN + 16), Some(OUTSIDE));
    assert_eq!(screen(&g), ["b", "c", "d"]);
}

// --- The randomized model test ------------------------------------------------

use crate::canvas::tests::Rng;

/// Random input of the kinds a shell produces, escape sequences included.
fn random_input(rng: &mut Rng) -> String {
    const TEXT: [&str; 8] = ["hello ", "Grüezi ", "x", "€§£", "│─┼", "\u{fffd}", "中", "abcdefghijklmnopqrstuvwxyz"];
    match rng.below(24) {
        0..=5 => TEXT[rng.below(TEXT.len() as u32) as usize].into(),
        6 => "\n".into(),
        7 => "\r".into(),
        8 => "\x08".into(),
        9 => "\t".into(),
        10 => format!("\x1b[{};{}H", rng.below(12), rng.below(30)),
        11 => format!("\x1b[{}{}", rng.below(5), ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'd'][rng.below(8) as usize]),
        12 => format!("\x1b[{}{}", rng.below(4), ['J', 'K'][rng.below(2) as usize]),
        13 => format!("\x1b[{}{}", rng.below(4), ['@', 'P', 'X', 'L', 'M', 'S', 'T'][rng.below(7) as usize]),
        14 => format!("\x1b[{};{}m", [0, 1, 4, 7, 9, 22, 31, 44, 92, 101][rng.below(10) as usize], 30 + rng.below(8)),
        15 => format!("\x1b[38;5;{}m", rng.below(256)),
        16 => ["\x1b[?25l", "\x1b[?25h", "\x1b[?7l", "\x1b[?7h"][rng.below(4) as usize].into(),
        17 => ["\x1b7", "\x1b8", "\x1bM", "\x1bD", "\x1bE"][rng.below(5) as usize].into(),
        18 => "\x1b[2J\x1b[H".into(),
        19 => "\n\n\n\n\n\n\n\n\n\n\n\n".into(),
        _ => (0..rng.below(40)).map(|_| char::from(b' ' + rng.below(95) as u8)).collect(),
    }
}

/// Feed two grids the same random input; render one after every few steps
/// (incrementally, as a terminal would) and compare its canvas with a full
/// render of the other on a fresh canvas. Every pixel that changed must lie
/// inside the rectangle `render` reported, and the margin around the grid
/// must stay untouched.
#[test]
fn incremental_render_matches_full_render() {
    let mut rng = Rng(0xfeed_beef_0bad_cafe);
    for round in 0..60 {
        let font = if round % 3 == 2 { &SPLEEN_12X24 } else { &SPLEEN_8X16 };
        let (cols, rows) = (1 + rng.below(14) as usize, 1 + rng.below(9) as usize);
        let scrollback = rng.below(6) as usize;
        let mut cells_a = storage(cols, rows, scrollback);
        let mut cells_b = storage(cols, rows, scrollback);
        let mut a = TextGrid::new(&mut cells_a, cols, rows).unwrap();
        let mut b = TextGrid::new(&mut cells_b, cols, rows).unwrap();
        let p = Palette::DEFAULT;
        let (mut buf_a, w, h) = canvas_buf(cols, rows, font);
        let (mut buf_b, _, _) = canvas_buf(cols, rows, font);
        // Everything since the last render, for the failure message.
        let mut log: Vec<String> = Vec::new();
        for step in 0..150 {
            match rng.below(30) {
                0 => {
                    let lines = rng.range(-3, 4) as isize;
                    log.push(format!("scroll_view({lines})"));
                    a.scroll_view(lines);
                    b.scroll_view(lines);
                }
                1 => {
                    let shape = [CursorShape::Block, CursorShape::Underline, CursorShape::Bar][rng.below(3) as usize];
                    log.push(format!("{shape:?}"));
                    a.set_cursor_shape(shape);
                    b.set_cursor_shape(shape);
                }
                2 => {
                    let visible = rng.below(2) == 0;
                    log.push(format!("cursor visible {visible}"));
                    a.set_cursor_visible(visible);
                    b.set_cursor_visible(visible);
                }
                _ => {
                    let s = random_input(&mut rng);
                    log.push(format!("{s:?}"));
                    // Byte-wise for one grid, as a string for the other.
                    a.write_bytes(s.as_bytes());
                    b.write_str(&s);
                }
            }
            if rng.below(3) != 0 {
                continue;
            }
            let before = buf_a.clone();
            let drawn = a.render(&mut canvas(&mut buf_a, w, h), MARGIN, MARGIN, font, &p);
            for (i, (old, new)) in before.chunks(4).zip(buf_a.chunks(4)).enumerate() {
                let (x, y) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
                assert!(
                    old == new || drawn.is_some_and(|r| r.contains(x, y)),
                    "round {round} step {step}: ({x}, {y}) changed outside the reported {drawn:?}"
                );
            }
            b.invalidate();
            let mut cb = canvas(&mut buf_b, w, h);
            cb.clear(OUTSIDE);
            b.render(&mut cb, MARGIN, MARGIN, font, &p);
            if buf_a != buf_b {
                let ca = canvas(&mut buf_a, w, h);
                let cb = canvas(&mut buf_b, w, h);
                let first = (0..h as i32)
                    .flat_map(|y| (0..w as i32).map(move |x| (x, y)))
                    .find(|&(x, y)| ca.pixel(x, y) != cb.pixel(x, y));
                panic!(
                    "round {round} ({cols}x{rows}, scrollback {scrollback}) step {step}: incremental render differs at {first:?} \
                     (cell {:?}); screen {:?}, cursor {:?}, view {}; input since the last render: {log:?}",
                    first.map(|(x, y)| ((x - MARGIN) / font.width() as i32, (y - MARGIN) / font.height() as i32)),
                    screen(&a),
                    a.cursor(),
                    a.view_offset()
                );
            }
            assert_eq!(screen(&a), screen(&b));
            log.clear();
        }
    }
}

/// Plain text and the basic controls against a straightforward model: all
/// lines in a list, the screen being the last `rows` of them.
#[test]
fn text_matches_a_simple_model() {
    let mut rng = Rng(0x7e47);
    for _ in 0..200 {
        let (cols, rows, scrollback) = (1 + rng.below(9) as usize, 1 + rng.below(6) as usize, rng.below(5) as usize);
        let mut cells = storage(cols, rows, scrollback);
        let mut g = TextGrid::new(&mut cells, cols, rows).unwrap();
        let mut lines: Vec<Vec<char>> = vec![vec![' '; cols]; rows];
        let (mut col, mut row, mut pending) = (0usize, 0usize, false);
        let mut input = String::new();
        for _ in 0..200 {
            let c = match rng.below(10) {
                0 => '\n',
                1 => '\r',
                2 => '\x08',
                3 => '\t',
                _ => char::from(b'a' + rng.below(26) as u8),
            };
            input.push(c);
            g.write_char(c);
            match c {
                '\n' => {
                    if row + 1 == rows {
                        lines.push(vec![' '; cols]);
                    } else {
                        row += 1;
                    }
                    (col, pending) = (0, false);
                }
                '\r' => (col, pending) = (0, false),
                '\x08' => (col, pending) = (col.saturating_sub(1), false),
                '\t' => (col, pending) = ((col / 8 + 1).saturating_mul(8).min(cols - 1), false),
                c => {
                    if pending {
                        col = 0;
                        if row + 1 == rows {
                            lines.push(vec![' '; cols]);
                        } else {
                            row += 1;
                        }
                    }
                    let top = lines.len() - rows;
                    lines[top + row][col] = c;
                    pending = col + 1 == cols;
                    if !pending {
                        col += 1;
                    }
                }
            }
            let top = lines.len() - rows;
            let want_screen: Vec<String> = lines[top..].iter().map(|l| line_text_chars(l)).collect();
            assert_eq!(screen(&g), want_screen, "{cols}x{rows} after {input:?}");
            let kept = top.min(scrollback);
            let want_history: Vec<String> = lines[top - kept..top].iter().map(|l| line_text_chars(l)).collect();
            assert_eq!(history(&g), want_history, "{cols}x{rows}+{scrollback} after {input:?}");
            assert_eq!(g.cursor(), (col, row));
        }
    }
}

fn line_text_chars(l: &[char]) -> String {
    l.iter().collect::<String>().trim_end().into()
}
