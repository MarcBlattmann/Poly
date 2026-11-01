//! Input: keyboards and mice, from controller bytes and device events to
//! the event layer, `/dev/input/event*` and characters.
//!
//! ```text
//!  i8042 (IRQ 1) ──bytes──▶ Set1Decoder ──ScanEvent──┐
//!   i8042::init              (Set2Decoder)           │ keycode::code
//!  i8042 aux (IRQ 12) ─▶ ps2mouse::MouseDecoder ─────┤
//!  virtio-input ───────────── evdev events ──────────┤
//!                                                    ▼
//!                       hub::Hub: devices, state, frames, soft repeat, LEDs, grabs
//!                         │                                   │
//!                         ▼ client queues                     ▼ console keys (every keyboard)
//!                  evdev: /dev/input/event* bytes      KeyState ──KeyEvent──▶ Composer + Layout ──▶ Output
//!                  and EVIOC* ioctls                   modifiers, locks       shift levels, AltGr, dead keys
//! ```
//!
//! - [`key`]: [`Key`] (a physical key position), [`Modifiers`], [`Leds`]
//!   and [`KeyEvent`].
//! - [`set1`] and [`set2`]: the scancode set 1 decoder (what an i8042
//!   delivers with translation on) and the set 2 one (what a keyboard
//!   sends), with their encoders.
//! - [`codes`]: the evdev event types and codes; [`keycode`] maps
//!   [`Key`]s to key codes and back.
//! - [`KeyState`]: which keys are down, the modifiers and locks.
//! - [`layout`]: layouts as data tables ([`layout::US`], [`layout::DE_CH`],
//!   [`layout::DE`]), and [`Composer`], which applies one to key events:
//!   characters, dead keys and the special keys a line editor uses
//!   ([`Output`]); [`keysym`] gives a key's X11 keysym on a level.
//! - [`Keyboard`]: all of the above in one, fed one byte at a time (or one
//!   decoded key event, [`Keyboard::apply`]).
//! - [`hub`]: the event layer: devices, a queue per reader, frames,
//!   soft repeat, LEDs, grabs and the console's keys; [`evdev`] puts the
//!   Linux event device interface on top.
//! - [`i8042`]: the controller's registers and bring-up sequence (keyboard
//!   and mouse) behind an access trait; [`ps2mouse`] the mouse protocol
//!   (wheel detection, packets); [`ring::ByteRing`] carries bytes from
//!   an interrupt handler to a thread.
//!
//! Nothing here panics on input: every byte sequence is accepted (unknown
//! codes are ignored), and every wait for the controller is bounded. The
//! hub allocates only when devices and readers come and go.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod codes;
mod compose;
pub mod evdev;
pub mod hub;
pub mod i8042;
pub mod key;
pub mod keycode;
pub mod keysym;
pub mod layout;
pub mod ps2mouse;
pub mod ring;
pub mod set1;
pub mod set2;
mod state;
pub mod xkb;

pub use compose::{Composer, Output, Special};
pub use key::{Key, KeyEvent, Leds, Modifiers};
pub use layout::{DeadKey, Layout};
pub use set1::{ScanEvent, Set1Decoder};
pub use set2::Set2Decoder;
pub use state::KeyState;

/// One key event and what it types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Input {
    pub event: KeyEvent,
    pub output: Output,
}

/// A keyboard on a set 1 byte stream: decoder, key state and composer with
/// the current layout.
#[derive(Clone, Debug)]
pub struct Keyboard {
    decoder: Set1Decoder,
    state: KeyState,
    composer: Composer,
    layout: &'static Layout,
}

impl Keyboard {
    /// No key down, every lock off.
    pub const fn new(layout: &'static Layout) -> Keyboard {
        let state = KeyState::with_alt_gr(layout.has_alt_gr);
        Keyboard { decoder: Set1Decoder::new(), state, composer: Composer::new(), layout }
    }

    pub fn layout(&self) -> &'static Layout {
        self.layout
    }

    /// Switch layouts. A pending dead key is dropped; keys held stay held.
    pub fn set_layout(&mut self, layout: &'static Layout) {
        self.layout = layout;
        self.state.set_alt_gr(layout.has_alt_gr);
        self.composer.reset();
    }

    /// Feed one byte from the controller; returns the key event it
    /// completes and what that types.
    pub fn feed(&mut self, byte: u8) -> Option<Input> {
        let scan = self.decoder.feed(byte)?;
        Some(self.apply(scan))
    }

    /// Take a key event decoded elsewhere (the event layer's console keys,
    /// a virtio keyboard): the key state and the layout as for bytes. A
    /// press of a key that is down is a repeat.
    pub fn apply(&mut self, scan: ScanEvent) -> Input {
        let event = self.state.apply(scan);
        let output = self.composer.translate(self.layout, &event);
        Input { event, output }
    }

    pub fn modifiers(&self) -> Modifiers {
        self.state.modifiers()
    }

    /// The LEDs that show the locks now.
    pub fn leds(&self) -> Leds {
        self.state.leds()
    }

    /// Set the locks (only the lock bits are used).
    pub fn set_locks(&mut self, locks: Modifiers) {
        self.state.set_locks(locks);
    }

    /// The keyboard was reset or bytes were lost: forget partial
    /// sequences, keys held and a pending dead key. The locks stay.
    pub fn resync(&mut self) {
        self.decoder.reset();
        self.state.release_all();
        self.composer.reset();
    }

    /// The dead key waiting for the next character.
    pub fn pending_dead_key(&self) -> Option<DeadKey> {
        self.composer.pending()
    }
}

#[cfg(test)]
mod tests;
