//! `poly-app`: what Poly's own native apps share (Files, Terminal; ADR
//! 0134). A native app is a separate native process (ADR 0125), a Wayland
//! client of the native compositor like any Linux app, so a crash of an app
//! never takes the desktop down; the compositor draws its frame (title bar,
//! buttons, border, shadow) as for every window.
//!
//! - [`proto`]: the Wayland client without I/O — the registry, one
//!   `xdg_toplevel` in one shared-memory buffer at the output's scale,
//!   pointer, keyboard, cursor shapes (`wp_cursor_shape_v1`), the clipboard
//!   (`wl_data_device`). Tested against `poly-wayland`'s own server in the
//!   same process.
//! - [`input`]: keys to characters on the desktop's layout, key repeat.
//! - [`ui`]: the Alpine Shell look (ADR 0127) for app windows, in the
//!   settings window's layout language, and a line editor and context menus.
//! - [`run`]: the event loop on `poly-rt` (the socket, the buffer, `poll`
//!   with the app's own handles, the clipboard's pipes, the theme following
//!   the settings file).

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod input;
pub mod proto;
pub mod run;
pub mod ui;

pub use input::{KeyInput, Mods};
pub use proto::CursorShape;
pub use run::{App, AppEvent, Cx, run};
pub use ui::{LineEdit, Look, Menu, MenuItem, Ui};

/// Mouse buttons (evdev codes).
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

#[cfg(test)]
mod tests;
