//! What the input stack plugs into: a seat's capabilities and the events
//! its devices produce, in the compositor's (logical) coordinates.
//!
//! The input layer (evdev, virtio-input, PS/2) is built separately; the
//! compositor implements [`InputSource`] over `/dev/input/event*` and its
//! loop feeds what it yields to [`crate::server::Server::handle_input`],
//! which routes it to clients through `wl_pointer` and `wl_keyboard` and
//! drives the frame (focus on click, the close button, moving by the title
//! bar; ADR 0107). The capabilities become `wl_seat.capabilities`.

use crate::wire::Fixed;
use alloc::collections::VecDeque;

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

/// Input waiting for a compositor frame. Consecutive pointer motions collapse
/// to their final position (absolute) or summed displacement (relative),
/// while buttons, keys and axes remain ordering barriers.
#[derive(Debug, Default)]
pub struct InputEventQueue {
    events: VecDeque<InputEvent>,
}

impl InputEventQueue {
    /// Queue an event, coalescing only with the immediately preceding motion.
    pub fn push(&mut self, event: InputEvent) {
        if let Some(last) = self.events.back_mut() {
            match last {
                InputEvent::PointerMotion { time, dx, dy } => {
                    if let InputEvent::PointerMotion { time: next_time, dx: next_dx, dy: next_dy } = event {
                        *time = next_time;
                        dx.0 = dx.0.saturating_add(next_dx.0);
                        dy.0 = dy.0.saturating_add(next_dy.0);
                        return;
                    }
                }
                InputEvent::PointerMotionAbsolute { .. } => {
                    if matches!(event, InputEvent::PointerMotionAbsolute { .. }) {
                        *last = event;
                        return;
                    }
                }
                _ => {}
            }
        }
        self.events.push_back(event);
    }

    /// Queue a batch, applying the same motion coalescing across its boundary.
    pub fn extend(&mut self, events: impl IntoIterator<Item = InputEvent>) {
        for event in events {
            self.push(event);
        }
    }

    /// Take the next event in order.
    pub fn pop_front(&mut self) -> Option<InputEvent> {
        self.events.pop_front()
    }
}

/// A source of input events: the input stack's side of the seat.
pub trait InputSource {
    /// The devices present now.
    fn capabilities(&self) -> Capabilities;
    /// The next event, if one is waiting.
    fn next_event(&mut self) -> Option<InputEvent>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_absolute_motion_keeps_only_the_latest_position() {
        let mut queue = InputEventQueue::default();
        queue.extend([
            InputEvent::PointerMotionAbsolute { time: 1, x: Fixed::from_int(10), y: Fixed::from_int(20) },
            InputEvent::PointerMotionAbsolute { time: 2, x: Fixed::from_int(30), y: Fixed::from_int(40) },
        ]);
        assert_eq!(
            queue.pop_front(),
            Some(InputEvent::PointerMotionAbsolute { time: 2, x: Fixed::from_int(30), y: Fixed::from_int(40) })
        );
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn relative_motion_sums_but_buttons_and_keys_keep_order() {
        let mut queue = InputEventQueue::default();
        queue.extend([
            InputEvent::PointerMotion { time: 1, dx: Fixed::from_int(3), dy: Fixed::from_int(-2) },
            InputEvent::PointerMotion { time: 2, dx: Fixed::from_int(4), dy: Fixed::from_int(5) },
            InputEvent::PointerButton { time: 3, button: BTN_LEFT, pressed: true },
            InputEvent::PointerMotion { time: 4, dx: Fixed::from_int(1), dy: Fixed::from_int(0) },
            InputEvent::Key { time: 5, key: 30, pressed: true },
        ]);
        assert_eq!(
            queue.pop_front(),
            Some(InputEvent::PointerMotion { time: 2, dx: Fixed::from_int(7), dy: Fixed::from_int(3) })
        );
        assert_eq!(queue.pop_front(), Some(InputEvent::PointerButton { time: 3, button: BTN_LEFT, pressed: true }));
        assert_eq!(
            queue.pop_front(),
            Some(InputEvent::PointerMotion { time: 4, dx: Fixed::from_int(1), dy: Fixed::from_int(0) })
        );
        assert_eq!(queue.pop_front(), Some(InputEvent::Key { time: 5, key: 30, pressed: true }));
        assert_eq!(queue.pop_front(), None);
    }
}
