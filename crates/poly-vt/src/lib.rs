//! The terminal emulator core of Poly's Terminal app.
//!
//! [`Terminal`] is a resizable VT100/VT220/xterm screen model: feed it what
//! the shell writes to the pty ([`Terminal::feed`]), draw its cells
//! ([`Terminal::visible_row`], [`Terminal::cursor`], [`Terminal::take_damage`])
//! and write back what it has to answer ([`Terminal::take_replies`]). It
//! covers what `TERM=xterm-256color` programs (bash, BusyBox ash and vi,
//! less, top, htop, vim, nano, tmux) use: cursor movement and erasing,
//! scroll regions, insert/delete, 256 and 24-bit colours, the alternate
//! screen, autowrap with the last-column flag, wide characters, DEC line
//! drawing, scrollback with a scrollable view, bracketed paste, mouse and
//! focus reporting, and the status and identification requests.
//!
//! [`keys`] turns key presses, pasted text and mouse events into the bytes
//! the program expects; [`palette`] gives the 256-colour palette.
//!
//! Written from public documentation only: ECMA-48, the DEC VT100/VT220
//! manuals (vt100.net), xterm's "ctlseqs" document and terminfo. `no_std`
//! with `alloc`; no input can make it panic.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod cell;
mod charset;
pub mod keys;
pub mod palette;
mod parser;
mod terminal;
mod width;

#[cfg(test)]
mod tests;

pub use cell::{Attrs, Cell, Color};
pub use terminal::{Cursor, CursorShape, Damage, MAX_COLS, MAX_ROWS, Modes, MouseMode, Terminal};
pub use width::char_width;
