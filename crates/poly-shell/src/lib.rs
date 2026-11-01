//! Poly's desktop shell (ADR 0107, ADR 0117): the look of the owner's
//! Alpine Shell re-created for Poly's compositor.
//!
//! - [`shell::Shell`]: the **ridge** (a slim status strip along the top:
//!   a menu glyph, the unread notifications, the network and volume, the
//!   clock and date in the configured time zone; nothing hangs from it),
//!   the **launcher** (an input box in the middle of the screen; typing
//!   filters the apps, Enter starts one, Escape hides it; it shows on an
//!   empty desktop), the **notifications** (toasts in the top right, a
//!   history in the middle), the **window switcher** (Alt+Tab), the
//!   **snap preview** and the **wallpaper**, drawn as two compositor
//!   layers, below and above the windows, with only what changed drawn
//!   again; and the contents of the **settings window**.
//! - [`apps`]: `.desktop` files (the Desktop Entry Specification) and the
//!   built-in apps; [`search`]: matching and ranking; [`launcher`]: the
//!   launcher's state and keys; [`clock`]: dates and the clock's text;
//!   [`tz`]: time zones; [`notify`]: the notification queue and history;
//!   [`switcher`]: the switcher's order, state and cards; [`wm`]: snap
//!   zones and Super+arrow; [`settings`]: the settings and their file;
//!   [`settings_ui`]: the settings window; [`draw`]: round rectangles,
//!   translucency and text (`poly-font`); [`theme`]: the measurements.
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
pub mod notify;
pub mod search;
pub mod settings;
pub mod settings_ui;
pub mod shell;
pub mod switcher;
pub mod theme;
pub mod tz;
pub mod wm;

pub use apps::App;
pub use launcher::Key;
pub use shell::{Network, Press, Shell};
