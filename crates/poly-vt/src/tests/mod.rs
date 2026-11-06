//! Tests of the whole terminal, through `feed` and the public API.

mod apps;
mod basics;
mod csi;
mod screens;
mod sgr;
mod text;

use crate::{Cell, Terminal};
use std::string::String;
use std::vec::Vec;

/// A terminal fed `s`.
fn term(cols: u16, rows: u16, s: &str) -> Terminal {
    let mut t = Terminal::new(cols, rows, 100);
    t.feed(s.as_bytes());
    t
}

/// Display row `r` as text (continuation cells skipped, padded to the full
/// width so positions are visible).
fn row(t: &Terminal, r: u16) -> String {
    let mut s: String = t.visible_row(r).iter().filter(|c| c.width != 0).map(|c| c.ch).collect();
    let shown = t.visible_row(r).iter().filter(|c| c.width != 0).count();
    let wide = t.visible_row(r).iter().filter(|c| c.width == 2).count();
    for _ in shown + wide..usize::from(t.cols()) {
        s.push(' ');
    }
    s
}

/// All display rows, trailing blanks trimmed.
fn rows(t: &Terminal) -> Vec<String> {
    (0..t.rows()).map(|r| row(t, r).trim_end().into()).collect()
}

fn cell(t: &Terminal, r: u16, c: usize) -> Cell {
    t.visible_row(r).get(c).copied().unwrap_or(Cell::BLANK)
}

fn cursor(t: &Terminal) -> (u16, u16) {
    let c = t.cursor();
    (c.row, c.col)
}
