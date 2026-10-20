//! The kernel's screen terminal, without the machine.
//!
//! The kernel (`kernel/src/terminal.rs`) puts a terminal on the display: a
//! status bar on the top row and a [`TextGrid`](poly_gfx::TextGrid) below
//! it, a kernel shell reading the keyboard and writing into the grid, and
//! the kernel log mirrored into the same grid. Everything here is the part
//! of that which does not touch a device, so it is tested on the host:
//!
//! - [`Screen`]: where things go on a display of a given size (font, grid
//!   size, status bar, how much scrollback fits the memory budget).
//! - [`status`]: the status bar's text ([`status::StatusInfo`] formatted to
//!   the screen's width) and how it is drawn.
//! - [`keys`]: a key event as the bytes a terminal sends for it (xterm
//!   conventions), which is what the line editor's `TermDecoder` reads, and
//!   the keys the terminal keeps for itself (Shift+Page Up / Page Down
//!   scroll the view into the scrollback).
//! - [`LogRing`]: the kernel log's last bytes, overwriting the oldest, read
//!   by position so a reader that fell behind knows how much it lost.
//! - [`Terminal`]: the grid plus the rules that join shell and log output
//!   on one screen: log output erases the prompt row and asks the shell to
//!   draw it again below (as the serial console does), log text in its own
//!   colour, the view snapping back on typing, and rendering grid and
//!   status bar onto a canvas.
//!
//! Nothing here allocates or panics on any input.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod keys;
mod log_ring;
mod screen;
pub mod status;
mod terminal;

pub use log_ring::{LogRead, LogRing};
pub use screen::{CELL_BUDGET_BYTES, SCROLLBACK_LINES, Screen};
pub use terminal::{LOG_STYLE, Terminal};
