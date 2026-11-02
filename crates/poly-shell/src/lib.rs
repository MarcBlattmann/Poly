//! Poly's desktop shell (ADR 0107, ADR 0117, ADR 0126): the look of the
//! owner's Alpine Shell re-created for Poly's compositor.
//!
//! - [`shell::Shell`]: the **ridge** (a strip along the top: the system
//!   menu's circle, the unread notifications, the network and volume, the
//!   clock and date in the configured time zone) and the **surfaces that
//!   hang from it**, one at a time with Alpine's shoulders and animations
//!   ([`chrome`]): the **launcher** (typing filters the apps, Enter starts
//!   one; it shows on an empty desktop), the **system menu**, the **control
//!   centre**, the **calendar**, the **notification history**, the
//!   **notices** at the top right and the **window switcher** (Alt+Tab);
//!   the **snap preview** and the **wallpaper**, drawn as two compositor
//!   layers, below and above the windows, with only what changed drawn
//!   again; and the contents of the **settings window**.
//! - [`apps`]: `.desktop` files (the Desktop Entry Specification) and the
//!   built-in apps; [`search`]: matching and ranking; [`launcher`]: the
//!   launcher's state and keys; [`clock`]: dates and the clock's text;
//!   [`tz`]: time zones; [`notify`]: the notification queue and history;
//!   [`switcher`]: the switcher's order, state and cards; [`wm`]: snap
//!   zones and Super+arrow; [`settings`]: the settings and their file;
//!   [`settings_ui`]: the settings window; [`draw`]: text in Alpine's fonts
//!   (`poly-font`) and shapes; [`icons`]: Lucide's icons; [`theme`]: the
//!   measurements and tokens.
//!
//! `no_std` with `alloc` and host-tested: the compositor
//! (`userland/poly-compositor`) owns the I/O (reading desktop files and
//! fonts, the clock, spawning apps) and the shell only decides and draws.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod apps;
pub mod chrome;
pub mod clock;
pub mod draw;
pub mod icons;
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
pub use shell::{ControlView, Network, Panel, Power, Press, Shell, Surface};
