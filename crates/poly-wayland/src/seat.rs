//! What the input stack plugs into: a seat's capabilities and the events
//! its devices produce, in the compositor's (logical) coordinates.
//!
//! The input layer (evdev, virtio-input, PS/2) is built separately; it
//! implements [`InputSource`] and the compositor's loop feeds what it
//! yields to [`crate::server::Server::handle_input`]. Today that only
//! decides the seat's `wl_seat.capabilities` and drives the frame (raise
//! and activate on a click, the close button); routing events to clients
//! through `wl_pointer` and `wl_keyboard` comes with the input stack.

use crate::wire::Fixed;

/// `wl_seat.capability` bits (wayland.xml).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Capabilities(pub u32);

impl Capabilities {
    pub const NONE: Capabilities = Capabilities(0);
    pub const POINTER: Capabilities = Capabilities(1);
    pub const KEYBOARD: Capabilities = Capabilities(2);
    pub const TOUCH: Capabilities = Capabilities(4);

    pub const fn contains(self, other: Capabilities) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Capabilities) -> Capabilities {
        Capabilities(self.0 | other.0)
    }
}

/// `BTN_LEFT` from linux/input-event-codes.h: buttons are evdev codes.
pub const BTN_LEFT: u32 = 0x110;

/// One input event. Times are milliseconds on the compositor's clock;
/// keys and buttons are evdev codes (linux/input-event-codes.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// The pointer moved by (`dx`, `dy`) logical pixels.
    PointerMotion {
        time: u32,
        dx: Fixed,
        dy: Fixed,
    },
    /// The pointer is at (`x`, `y`) on the output (tablets, a VM's
    /// absolute pointer).
    PointerMotionAbsolute {
        time: u32,
        x: Fixed,
        y: Fixed,
    },
    PointerButton {
        time: u32,
        button: u32,
        pressed: bool,
    },
    /// Scrolling: `horizontal` or vertical, by `value` logical pixels.
    PointerAxis {
        time: u32,
        horizontal: bool,
        value: Fixed,
    },
    Key {
        time: u32,
        key: u32,
        pressed: bool,
    },
}

/// A source of input events: the input stack's side of the seat.
pub trait InputSource {
    /// The devices present now.
    fn capabilities(&self) -> Capabilities;
    /// The next event, if one is waiting.
    fn next_event(&mut self) -> Option<InputEvent>;
}
