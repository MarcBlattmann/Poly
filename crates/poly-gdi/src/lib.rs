//! GDI without the machine (ADR 0128, Decision 5).
//!
//! GDI draws in the app's process: `gdi32.dll` keeps one [`gdi::Gdi`] (the
//! process's objects and their handles) and maps its exports onto it;
//! `user32.dll` reaches it through `gdi32`'s private exports for window DCs,
//! `FillRect` and `DrawTextW`. Everything here is tested on the host over
//! memory the tests own.
//!
//! - [`bitmap`]: bitmaps, the DIB pixel formats and colour conversions,
//!   raster operations.
//! - [`gdi`]: objects, DCs, `BitBlt`/`PatBlt`/`Rectangle`/pixels/text.
//! - [`font`]: the stock fonts' metrics and widths, and the system font's
//!   glyphs (`sysfont`).
//! - [`text`]: `DrawTextW`'s layout.
//! - [`region`]: region algebra (rectangles in canonical bands), for
//!   `HRGN`s, clipping, and the window manager's update regions.
//!
//! Behaviour follows the slice's spec sheets (`specs/windows/gdi32.dll/`,
//! `user32.dll/FillRect`, `DrawTextW`, `GetDC`, `ReleaseDC`).

#![no_std]

extern crate alloc;

pub mod bitmap;
pub mod font;
pub mod gdi;
pub mod region;
mod sysfont;
pub mod text;

#[cfg(test)]
mod tests;
