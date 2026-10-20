//! A tiny terminal for the tests: rows of characters and a cursor. It
//! understands exactly the output the crate documents and panics on
//! anything else, so a test also proves the editor stays inside that set.
//! Rows have no width limit (the editor does not wrap), and moving the
//! cursor left of column 0 panics instead of stopping there, because a
//! correct editor never does it.

use std::string::String;
use std::vec::Vec;

#[derive(Debug, Default)]
pub struct Term {
    rows: Vec<Vec<char>>,
    pub row: usize,
    pub col: usize,
}

impl Term {
    pub fn new() -> Term {
        Term { rows: std::vec![Vec::new()], row: 0, col: 0 }
    }

    /// The text of row `r`.
    pub fn row_text(&self, r: usize) -> String {
        self.rows.get(r).map(|r| r.iter().collect()).unwrap_or_default()
    }

    /// The row the cursor is on.
    pub fn current(&self) -> String {
        self.row_text(self.row)
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    pub fn feed(&mut self, s: &str) {
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            match c {
                '\x1b' => {
                    assert_eq!(chars.next(), Some('['), "only CSI sequences are allowed, in {s:?}");
                    let mut params = String::new();
                    let last = loop {
                        let c = chars.next().expect("unterminated control sequence");
                        if c.is_ascii_digit() || c == ';' {
                            params.push(c);
                        } else {
                            break c;
                        }
                    };
                    self.csi(&params, last);
                }
                '\n' => {
                    self.row += 1;
                    self.col = 0;
                    if self.row == self.rows.len() {
                        self.rows.push(Vec::new());
                    }
                }
                '\r' => self.col = 0,
                c if c.is_control() => panic!("control character {c:?} in output {s:?}"),
                c => {
                    let row = &mut self.rows[self.row];
                    if row.len() <= self.col {
                        row.resize(self.col + 1, ' ');
                    }
                    row[self.col] = c;
                    self.col += 1;
                }
            }
        }
    }

    fn csi(&mut self, params: &str, last: char) {
        let n = || if params.is_empty() { 1 } else { params.parse::<usize>().expect("one numeric parameter") };
        match last {
            'D' => self.col = self.col.checked_sub(n()).expect("cursor moved left of column 0"),
            'C' => self.col += n(),
            'K' => {
                assert!(params.is_empty() || params == "0", "only ESC [ K erases a line");
                self.rows[self.row].truncate(self.col);
            }
            'J' => {
                assert_eq!(params, "2", "only ESC [ 2 J erases the screen");
                self.rows.iter_mut().for_each(Vec::clear);
            }
            'H' => {
                assert!(params.is_empty(), "only ESC [ H positions the cursor");
                (self.row, self.col) = (0, 0);
            }
            'm' => {} // colors
            _ => panic!("unsupported control sequence ESC [ {params} {last}"),
        }
    }
}
