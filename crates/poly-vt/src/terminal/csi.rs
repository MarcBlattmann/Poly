//! Control sequences (`CSI ...`): what each one does.
//!
//! | Sequence | Does |
//! |---|---|
//! | `@` ICH, `P` DCH, `X` ECH | insert / delete / erase `n` cells |
//! | `A` `B` `C` `D` CUU CUD CUF CUB | cursor up / down / right / left (up and down stop at the margins) |
//! | `E` CNL, `F` CPL | down / up `n` lines, to column 1 |
//! | `G` CHA, `` ` `` HPA, `a` HPR | to column `n` / right `n` |
//! | `d` VPA, `e` VPR | to row `n` / down `n` |
//! | `H`, `f` CUP | to row; column (origin mode: inside the region) |
//! | `I` CHT, `Z` CBT | forward / back `n` tab stops |
//! | `J` ED, `?J` | erase below, above, all, the scrollback (3) |
//! | `K` EL, `?K` | erase right, left, the line |
//! | `L` IL, `M` DL | insert / delete lines in the scroll region |
//! | `S` SU, `T` SD | scroll the region up / down |
//! | `b` REP | repeat the last character |
//! | `c` DA, `>c` DA2 | identify: VT220 with colour (`CSI ?62;22c`), `CSI >1;10;0c` |
//! | `g` TBC | clear the tab stop here (0) or all (3) |
//! | `h` / `l` SM / RM | `4` insert, `20` newline mode |
//! | `?h` / `?l` | DEC modes: 1, 5, 6, 7, 9, 12, 25, 47, 66, 1000, 1002, 1003, 1004, 1006, 1047, 1048, 1049, 2004 |
//! | `m` SGR | style and colours, below |
//! | `n` DSR, `?n` | `5`: `CSI 0n`; `6`: cursor position `CSI row;col R` (`CSI ?row;colR`) |
//! | `r` DECSTBM | set the scroll region, cursor home |
//! | `s`, `u` | save, restore the cursor |
//! | `t` XTWINOPS | `18`: reply the size `CSI 8;rows;cols t`; others ignored |
//! | `SP q` DECSCUSR | cursor shape and blinking |
//! | `!p` DECSTR | soft reset |
//! | `$p`, `?$p` DECRQM | report an ANSI / DEC mode: `CSI ?Ps;Pm$y` |
//! | `>q` XTVERSION | `DCS >| poly-vt <version> ST` |
//!
//! SGR: 0 reset; 1 bold, 2 faint, 3 italic, 4 underline (`4:0` off, `4:1`
//! to `4:5` on), 5 / 6 blink, 7 inverse, 8 hidden, 9 crossed out, 21
//! (double) underline; 22-29 turn those off; 30-37 / 40-47 and 90-97 /
//! 100-107 the 16 colours, 39 / 49 the default, `38;5;n` / `38;2;r;g;b`
//! and the colon forms `38:5:n`, `38:2::r:g:b`, `38:2:r:g:b` (same for 48);
//! underline colours (58, 59) are skipped.

use super::{MouseMode, Terminal};
use crate::cell::{Attrs, Color};
use crate::parser::Csi;
use alloc::format;

impl Terminal {
    pub(super) fn dispatch_csi(&mut self, csi: &Csi) {
        let n = usize::from(csi.or(0, 1));
        match (csi.private, csi.intermediates(), csi.final_byte) {
            (0, [], b'@') => self.insert_cells(n),
            (0, [], b'A') => self.cursor_up(n),
            (0, [], b'B') | (0, [], b'e') => self.cursor_down(n),
            (0, [], b'C') | (0, [], b'a') => self.cursor_right(n),
            (0, [], b'D') => self.cursor_left(n),
            (0, [], b'E') => {
                self.cursor_down(n);
                self.carriage_return();
            }
            (0, [], b'F') => {
                self.cursor_up(n);
                self.carriage_return();
            }
            (0, [], b'G') | (0, [], b'`') => self.goto(self.cur.row, n - 1),
            (0, [], b'H') | (0, [], b'f') => {
                self.goto_origin(usize::from(csi.or(0, 1)) - 1, usize::from(csi.or(1, 1)) - 1)
            }
            (0, [], b'd') => {
                let col = self.cur.col;
                self.goto_origin(n - 1, col);
            }
            (0, [], b'I') => self.tab(n),
            (0, [], b'Z') => self.back_tab(n),
            (0 | b'?', [], b'J') => self.erase_display(csi.get(0)),
            (0 | b'?', [], b'K') => self.erase_line(csi.get(0)),
            (0, [], b'L') => self.insert_lines(n),
            (0, [], b'M') => self.delete_lines(n),
            (0, [], b'P') => self.delete_cells(n),
            (0, [], b'S') => self.scroll_up(self.top, self.bottom, n, true),
            (0, [], b'T') if csi.len() <= 1 => self.scroll_down(self.top, self.bottom, n),
            (0, [], b'X') => self.erase_chars(n),
            (0, [], b'b') => self.repeat(n),
            (0, [], b'c') if csi.get(0) == 0 => self.reply("\x1b[?62;22c"),
            (b'>', [], b'c') if csi.get(0) == 0 => self.reply("\x1b[>1;10;0c"),
            (0, [], b'g') => match csi.get(0) {
                0 => {
                    if let Some(t) = self.tabs.get_mut(self.cur.col) {
                        *t = false;
                    }
                }
                3 => self.tabs.fill(false),
                _ => {}
            },
            (0, [], b'h') => csi.params().iter().for_each(|&p| self.set_ansi_mode(p, true)),
            (0, [], b'l') => csi.params().iter().for_each(|&p| self.set_ansi_mode(p, false)),
            (b'?', [], b'h') => csi.params().iter().for_each(|&p| self.set_dec_mode(p, true)),
            (b'?', [], b'l') => csi.params().iter().for_each(|&p| self.set_dec_mode(p, false)),
            (0, [], b'm') => self.sgr(csi),
            (0, [], b'n') => self.status_report(csi.get(0), false),
            (b'?', [], b'n') => self.status_report(csi.get(0), true),
            (0, [], b'r') => self.set_scroll_region(csi),
            (0, [], b's') => self.save_cursor(),
            (0, [], b'u') => self.restore_cursor(),
            (0, [], b't') => {
                if csi.get(0) == 18 {
                    let s = format!("\x1b[8;{};{}t", self.rows, self.cols);
                    self.reply(&s);
                }
            }
            (0, [b' '], b'q') => self.set_cursor_style(csi.get(0)),
            (0, [b'!'], b'p') => self.soft_reset(),
            (0 | b'?', [b'$'], b'p') => self.report_mode(csi.get(0), csi.private == b'?'),
            (b'>', [], b'q') if csi.get(0) == 0 => {
                let s = format!("\x1bP>|poly-vt {}\x1b\\", env!("CARGO_PKG_VERSION"));
                self.reply(&s);
            }
            _ => {}
        }
    }

    fn set_ansi_mode(&mut self, p: u16, on: bool) {
        match p {
            4 => self.modes.insert = on,
            20 => self.modes.newline = on,
            _ => {}
        }
    }

    fn ansi_mode(&self, p: u16) -> Option<bool> {
        match p {
            4 => Some(self.modes.insert),
            20 => Some(self.modes.newline),
            _ => None,
        }
    }

    fn set_dec_mode(&mut self, p: u16, on: bool) {
        let mouse = |m: MouseMode, cur: MouseMode| {
            if on {
                m
            } else if cur == m {
                MouseMode::None
            } else {
                cur
            }
        };
        match p {
            1 => self.modes.app_cursor = on,
            5 => {
                if self.modes.reverse_video != on {
                    self.damage_all = true;
                }
                self.modes.reverse_video = on;
            }
            6 => {
                self.modes.origin = on;
                self.goto_origin(0, 0);
            }
            7 => {
                self.modes.autowrap = on;
                if !on {
                    self.cur.pending_wrap = false;
                }
            }
            9 | 1000 => self.modes.mouse = mouse(MouseMode::Press, self.modes.mouse),
            1002 => self.modes.mouse = mouse(MouseMode::Drag, self.modes.mouse),
            1003 => self.modes.mouse = mouse(MouseMode::Any, self.modes.mouse),
            12 => self.cursor_blink = on,
            25 => self.cursor_visible = on,
            47 => self.set_alt_screen(on, false),
            66 => self.modes.app_keypad = on,
            1004 => self.modes.focus_events = on,
            1006 => self.modes.sgr_mouse = on,
            1047 => self.set_alt_screen(on, !on),
            1048 => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if on {
                    if !self.modes.alt_screen {
                        self.save_cursor();
                    }
                    self.set_alt_screen(true, true);
                } else if self.modes.alt_screen {
                    self.set_alt_screen(false, false);
                    self.restore_cursor();
                }
            }
            2004 => self.modes.bracketed_paste = on,
            _ => {}
        }
    }

    fn dec_mode(&self, p: u16) -> Option<bool> {
        let m = &self.modes;
        Some(match p {
            1 => m.app_cursor,
            5 => m.reverse_video,
            6 => m.origin,
            7 => m.autowrap,
            9 | 1000 => m.mouse == MouseMode::Press,
            1002 => m.mouse == MouseMode::Drag,
            1003 => m.mouse == MouseMode::Any,
            12 => self.cursor_blink,
            25 => self.cursor_visible,
            47 | 1047 | 1049 => m.alt_screen,
            66 => m.app_keypad,
            1004 => m.focus_events,
            1006 => m.sgr_mouse,
            2004 => m.bracketed_paste,
            _ => return None,
        })
    }

    /// DECRQM: `CSI ? Ps ; Pm $ y`, Pm 1 set, 2 reset, 0 not recognised.
    fn report_mode(&mut self, p: u16, dec: bool) {
        let state = if dec { self.dec_mode(p) } else { self.ansi_mode(p) };
        let pm = match state {
            Some(true) => 1,
            Some(false) => 2,
            None => 0,
        };
        let s = format!("\x1b[{}{p};{pm}$y", if dec { "?" } else { "" });
        self.reply(&s);
    }

    fn status_report(&mut self, p: u16, dec: bool) {
        match p {
            5 if !dec => self.reply("\x1b[0n"),
            6 => {
                let row = if self.modes.origin { self.cur.row.saturating_sub(self.top) } else { self.cur.row };
                let s = format!("\x1b[{}{};{}R", if dec { "?" } else { "" }, row + 1, self.cur.col + 1);
                self.reply(&s);
            }
            _ => {}
        }
    }

    fn set_scroll_region(&mut self, csi: &Csi) {
        let top = usize::from(csi.or(0, 1)) - 1;
        let bottom = usize::from(csi.or(1, self.rows as u16)).min(self.rows) - 1;
        if top < bottom {
            self.top = top;
            self.bottom = bottom;
            self.goto_origin(0, 0);
        }
    }

    fn set_cursor_style(&mut self, p: u16) {
        use super::CursorShape::*;
        let (shape, blink) = match p {
            0 | 1 => (Block, true),
            2 => (Block, false),
            3 => (Underline, true),
            4 => (Underline, false),
            5 => (Bar, true),
            6 => (Bar, false),
            _ => return,
        };
        self.cursor_shape = shape;
        self.cursor_blink = blink;
    }

    fn sgr(&mut self, csi: &Csi) {
        let params = csi.params();
        let len = params.len();
        let mut i = 0;
        while i < len {
            let p = params[i];
            // The sub-parameters of this one: `i + 1 .. end`.
            let mut end = i + 1;
            while end < len && csi.is_sub(end) {
                end += 1;
            }
            let subs = &params[i + 1..end];
            let attrs = &mut self.pen.attrs;
            match p {
                0 => self.pen = Default::default(),
                1 => attrs.insert(Attrs::BOLD),
                2 => attrs.insert(Attrs::FAINT),
                3 => attrs.insert(Attrs::ITALIC),
                4 => attrs.set(Attrs::UNDERLINE, subs.first().is_none_or(|&s| s != 0)),
                5 | 6 => attrs.insert(Attrs::BLINK),
                7 => attrs.insert(Attrs::INVERSE),
                8 => attrs.insert(Attrs::HIDDEN),
                9 => attrs.insert(Attrs::STRIKE),
                21 => attrs.insert(Attrs::UNDERLINE),
                22 => attrs.remove(Attrs::BOLD | Attrs::FAINT),
                23 => attrs.remove(Attrs::ITALIC),
                24 => attrs.remove(Attrs::UNDERLINE),
                25 => attrs.remove(Attrs::BLINK),
                27 => attrs.remove(Attrs::INVERSE),
                28 => attrs.remove(Attrs::HIDDEN),
                29 => attrs.remove(Attrs::STRIKE),
                30..=37 => self.pen.fg = Color::Indexed((p - 30) as u8),
                39 => self.pen.fg = Color::Default,
                40..=47 => self.pen.bg = Color::Indexed((p - 40) as u8),
                49 => self.pen.bg = Color::Default,
                90..=97 => self.pen.fg = Color::Indexed((p - 90 + 8) as u8),
                100..=107 => self.pen.bg = Color::Indexed((p - 100 + 8) as u8),
                38 | 48 | 58 => {
                    let (color, used) = if subs.is_empty() {
                        // `38;5;n` / `38;2;r;g;b`: the arguments are the
                        // following parameters.
                        let (c, used) = extended_color(&params[end..], false);
                        (c, used)
                    } else {
                        (extended_color(subs, true).0, 0)
                    };
                    end += used;
                    if let Some(c) = color {
                        match p {
                            38 => self.pen.fg = c,
                            48 => self.pen.bg = c,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            i = end;
        }
    }
}

/// Parse the arguments of SGR 38 / 48 (`5;n` or `2;r;g;b`, or with `:`
/// `5:n`, `2:cs:r:g:b`, `2:r:g:b`). Returns the colour and how many of the
/// arguments it used.
fn extended_color(args: &[u16], colon: bool) -> (Option<Color>, usize) {
    let byte = |v: u16| v.min(255) as u8;
    match args.first() {
        Some(5) => match args.get(1) {
            Some(&n) => (Some(Color::Indexed(byte(n))), 2),
            None => (None, args.len()),
        },
        Some(2) => {
            let rgb = if colon && args.len() >= 5 { args.get(2..5) } else { args.get(1..4) };
            match rgb {
                Some(&[r, g, b]) => (Some(Color::Rgb(byte(r), byte(g), byte(b))), 4),
                _ => (None, args.len()),
            }
        }
        _ => (None, usize::from(!args.is_empty())),
    }
}
