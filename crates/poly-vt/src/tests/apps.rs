//! Output shaped like what real programs write (written by hand from what
//! their terminfo entries for xterm-256color make them send).

use super::*;
use crate::{Attrs, Color};

/// A pager on a 20x5 terminal: alternate screen, a page of text, a status
/// line in reverse video, one line forward (LF at the bottom of a scroll
/// region excluding the status line), one line back (reverse index at the
/// top), and quitting back to the shell.
#[test]
fn pager_session() {
    let mut t = Terminal::new(20, 5, 100);
    t.feed(b"$ less file\r\n");
    t.feed(b"\x1b[?1049h\x1b[22;0;0t\x1b[?1h\x1b=\r");
    t.feed(b"\x1b[H\x1b[2J");
    t.feed(b"line 1\r\nline 2\r\nline 3\r\nline 4\r\n\x1b[7mfile\x1b[27m\x1b[K");
    assert_eq!(rows(&t), ["line 1", "line 2", "line 3", "line 4", "file"]);
    assert!(cell(&t, 4, 0).attrs.contains(Attrs::INVERSE));
    assert!(t.modes().app_cursor && t.modes().app_keypad);
    // Forward a line: the status line is redrawn after scrolling the text.
    t.feed(b"\x1b[1;4r\x1b[4;1H\nline 5\x1b[r\x1b[5;1H\x1b[K:");
    assert_eq!(rows(&t), ["line 2", "line 3", "line 4", "line 5", ":"]);
    // Back a line: reverse index at the top of the text region.
    t.feed(b"\x1b[1;4r\x1b[H\x1bMline 1\x1b[r\x1b[5;1H\x1b[K:");
    assert_eq!(rows(&t), ["line 1", "line 2", "line 3", "line 4", ":"]);
    assert_eq!(t.scrollback_len(), 0);
    // Quit.
    t.feed(b"\r\x1b[K\x1b[?1l\x1b>\x1b[?1049l\x1b[23;0;0t$ ");
    assert_eq!(rows(&t), ["$ less file", "$", "", "", ""]);
    assert_eq!(cursor(&t), (1, 2));
    assert!(!t.modes().app_cursor && !t.modes().app_keypad);
}

/// An editor: alternate screen, `~` lines in blue, a status line, inserting
/// a line in the middle with IL inside a scroll region, deleting a line,
/// cursor shape changes and title.
#[test]
fn editor_session() {
    let mut t = Terminal::new(12, 6, 100);
    t.feed(b"\x1b]2;vi notes\x07\x1b[?1049h\x1b[?25l\x1b[H\x1b[2J");
    t.feed(b"alpha\r\nbeta\r\ngamma\r\n\x1b[34m~\x1b[39m\r\n\x1b[34m~\x1b[39m\r\n\x1b[1m-- INSERT --\x1b[m");
    t.feed(b"\x1b[2;1H\x1b[?25h\x1b[6 q");
    assert_eq!(t.title(), "vi notes");
    assert_eq!(cell(&t, 3, 0).fg, Color::Indexed(4));
    assert!(cell(&t, 5, 0).attrs.contains(Attrs::BOLD));
    // `O` above beta: open a line with IL in the text region (rows 1-5).
    t.feed(b"\x1b[1;5r\x1b[2;1H\x1b[Lnew\x1b[r");
    assert_eq!(rows(&t), ["alpha", "new", "beta", "gamma", "~", "-- INSERT --"]);
    // `dd` on gamma: DL, and a `~` comes in at the bottom of the region.
    t.feed(b"\x1b[1;5r\x1b[4;1H\x1b[M\x1b[5;1H\x1b[34m~\x1b[39m\x1b[r");
    assert_eq!(rows(&t), ["alpha", "new", "beta", "~", "~", "-- INSERT --"]);
    // Delete a character inside a line and insert one.
    t.feed(b"\x1b[1;2H\x1b[P\x1b[4h\x1b[1;1HA\x1b[4l");
    assert_eq!(rows(&t)[0], "Aapha");
    t.feed(b"\x1b[2 q\x1b[?1049l");
    assert_eq!(t.cursor().shape, crate::CursorShape::Block);
    assert_eq!(rows(&t), ["", "", "", "", "", ""]);
}

/// A line editor redrawing a long command line that wraps: the shell
/// prints past the last column, moves up with CUU and erases with ED.
#[test]
fn line_editor_wraps_and_redraws() {
    let mut t = Terminal::new(10, 4, 100);
    t.feed(b"$ echo 0123456789");
    assert_eq!(rows(&t), ["$ echo 012", "3456789", "", ""]);
    assert!(t.line_wrapped(0));
    // Redraw from the prompt: CR, up one line, past the prompt, clear.
    t.feed(b"\r\x1b[A\x1b[2C\x1b[Jls");
    assert_eq!(rows(&t), ["$ ls", "", "", ""]);
    t.feed(b"\r\nfile\r\n$ ");
    assert_eq!(t.text((0, 0), (2, 9)), "$ ls\nfile\n$");
}

/// A status display drawing boxes with DEC line drawing and 256 colours,
/// as top/htop do on terminals without UTF-8 line drawing.
#[test]
fn box_drawing_screen() {
    let mut t = Terminal::new(8, 3, 0);
    t.feed(b"\x1b[H\x1b[2J\x1b(0lqqqqqqk\x1b(B\r\n\x1b(0x\x1b(B\x1b[38;5;208mCPU 9%\x1b[m\x1b(0x\x1b(B\r\n");
    t.feed(b"\x1b(0mqqqqqqj\x1b(B");
    assert_eq!(rows(&t), ["┌──────┐", "│CPU 9%│", "└──────┘"]);
    assert_eq!(cell(&t, 1, 1).fg, Color::Indexed(208));
    // Bottom-right corner written with a wrap pending, nothing scrolled.
    assert_eq!(cursor(&t), (2, 7));
}
