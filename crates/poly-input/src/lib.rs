//! Keyboard input, from controller bytes to characters.
//!
//! ```text
//!  i8042 (IRQ 1) ──bytes──▶ Set1Decoder ──ScanEvent──▶ KeyState ──KeyEvent──▶ Composer + Layout ──▶ Output
//!   i8042::init              E0 / E1 sequences          modifiers, locks,       shift levels, Caps / Num Lock,
//!   (polled bring-up)        Pause, Print Screen         auto-repeat             AltGr, dead keys, Ctrl
//! ```
//!
//! - [`key`]: [`Key`] (a physical key position), [`Modifiers`], [`Leds`]
//!   and [`KeyEvent`].
//! - [`set1`]: the scancode set 1 decoder (what an i8042 delivers with
//!   translation on) and its encoder.
//! - [`KeyState`]: which keys are down, the modifiers and locks.
//! - [`layout`]: layouts as data tables ([`layout::US`], [`layout::DE_CH`]),
//!   and [`Composer`], which applies one to key events: characters, dead
//!   keys and the special keys a line editor uses ([`Output`]).
//! - [`Keyboard`]: all of the above in one, fed one byte at a time.
//! - [`i8042`]: the controller's registers and bring-up sequence behind an
//!   access trait; [`ring::ByteRing`] carries bytes from the interrupt
//!   handler to a thread.
//!
//! Nothing here allocates or panics on input: every byte sequence is
//! accepted (unknown codes are ignored), and every wait for the controller
//! is bounded.

#![no_std]

#[cfg(test)]
extern crate std;

mod compose;
pub mod i8042;
pub mod key;
pub mod layout;
pub mod ring;
pub mod set1;
mod state;

pub use compose::{Composer, Output, Special};
pub use key::{Key, KeyEvent, Leds, Modifiers};
pub use layout::{DeadKey, Layout};
pub use set1::{ScanEvent, Set1Decoder};
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
        let event = self.state.apply(scan);
        let output = self.composer.translate(self.layout, &event);
        Some(Input { event, output })
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
