//! Drawing on linear framebuffers, for the kernel's display and terminal.
//!
//! - [`Canvas`] is a rectangle of pixels in memory the caller owns (a
//!   framebuffer mapping or a back buffer in RAM): width, height, stride in
//!   bytes and a [`PixelFormat`]. It clears and fills (clipped) rectangles,
//!   draws lines, copies pixels from another canvas ([`Canvas::blit`]) or
//!   within itself ([`Canvas::copy_within`], which is how a terminal
//!   scrolls), and draws text.
//! - [`Font`] is a fixed-cell bitmap font. Two are built in, both from Spleen
//!   by Frederic Cambus (BSD 2-Clause; `third_party/spleen/`):
//!   [`SPLEEN_8X16`] and [`SPLEEN_12X24`]. They cover ASCII, Latin-1 and
//!   Latin Extended-A, Greek and Cyrillic basics, box drawing, block
//!   elements and Braille; anything else draws as a box.
//! - [`TextGrid`] is a terminal screen: rows × columns of [`Cell`]s with a
//!   cursor, line wrap, scrolling into a scrollback buffer and the common
//!   ANSI / VT100 escape sequences (colours, cursor movement, erasing). It
//!   remembers which cells changed, so [`TextGrid::render`] redraws only
//!   those (and scrolls by copying pixels) onto a canvas at any offset.
//! - [`boot_screen`] is the picture the kernel shows when the display comes
//!   up, here so the boot test can render the same picture and compare it
//!   with a screenshot.
//!
//! Nothing here allocates: canvases and the text grid's cells live in
//! memory the caller provides, so the kernel decides where (and how much).
//! Nothing panics on any input either; drawing outside a canvas is clipped.

#![no_std]

pub mod boot_screen;
mod canvas;
mod color;
mod font;
mod geom;
mod grid;

pub use canvas::{Canvas, LayoutError};
pub use color::{Color, PixelFormat};
pub use font::{FONTS, Font, Glyph, SPLEEN_8X16, SPLEEN_12X24};
pub use geom::Rect;
pub use grid::{Attrs, Cell, CursorShape, GridError, MAX_ROWS, Paint, Palette, Style, TextGrid};
