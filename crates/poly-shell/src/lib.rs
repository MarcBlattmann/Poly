//! Poly's desktop shell, first version (ADR 0107): the look of the owner's
//! Alpine Shell re-created for Poly's compositor.
//!
//! - [`shell::Shell`]: the **ridge** (a slim status strip along the top:
//!   a menu glyph, a network and volume placeholder, the clock and date;
//!   no menus hang from it), the **launcher** (an input box in the middle
//!   of the screen; typing filters the apps, Enter starts one, Escape
//!   hides it; it shows on an empty desktop) and the **wallpaper**, drawn
//!   as two compositor layers, below and above the windows, with only
//!   what changed drawn again.
//! - [`apps`]: `.desktop` files (the Desktop Entry Specification) and the
//!   built-in apps; [`search`]: matching and ranking; [`launcher`]: the
//!   launcher's state and keys; [`clock`]: dates and the clock's text;
//!   [`draw`]: round rectangles, translucency and text (`poly-font`);
//!   [`theme`]: the measurements.
//!
//! `no_std` with `alloc` and host-tested: the compositor
//! (`userland/poly-compositor`) owns the I/O (reading desktop files and
//! fonts, the clock, spawning apps) and the shell only decides and draws.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod apps;
pub mod clock;
pub mod draw;
pub mod launcher;
pub mod search;
pub mod shell;
pub mod theme;

pub use apps::App;
pub use launcher::Key;
pub use shell::{Press, Shell};
