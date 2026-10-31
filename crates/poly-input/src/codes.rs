//! Event types and codes of the Linux input event interface (evdev).
//!
//! These numbers are ABI: they are what `struct input_event` carries, what
//! the `EVIOCGBIT` bitmaps index and what virtio-input devices send
//! (VIRTIO 1.2 §5.8: "the device uses the evdev event types and codes").
//! The event layer ([`crate::hub`]) uses them as its own vocabulary, so a
//! key code is the same number in every driver, in the hub and in
//! `/dev/input/event*`.
//!
//! Source: the public uapi header `linux/input-event-codes.h` and the
//! kernel's input documentation (`Documentation/input/event-codes.rst`).
//! Only the names Poly uses are listed.

/// Event types (`EV_*`).
pub mod ev {
    /// Separates events into frames (`SYN_REPORT`) and reports overruns.
    pub const SYN: u16 = 0x00;
    /// Keys and buttons: value 0 released, 1 pressed, 2 auto-repeated.
    pub const KEY: u16 = 0x01;
    /// Relative axes: mouse motion, wheels.
    pub const REL: u16 = 0x02;
    /// Absolute axes: tablets, touch screens.
    pub const ABS: u16 = 0x03;
    pub const MSC: u16 = 0x04;
    pub const SW: u16 = 0x05;
    /// LEDs, set by the host (keyboard lock lights).
    pub const LED: u16 = 0x11;
    pub const SND: u16 = 0x12;
    /// Auto-repeat parameters.
    pub const REP: u16 = 0x14;
    pub const FF: u16 = 0x15;
    pub const PWR: u16 = 0x16;
    pub const FF_STATUS: u16 = 0x17;
    pub const MAX: u16 = 0x1f;
    pub const CNT: usize = MAX as usize + 1;
}

/// `EV_SYN` codes.
pub mod syn {
    /// The end of a frame: the events since the last one happened at once.
    pub const REPORT: u16 = 0;
    pub const CONFIG: u16 = 1;
    pub const MT_REPORT: u16 = 2;
    /// The client's queue overran: ignore events up to and including the
    /// next `SYN_REPORT` and query the state with the `EVIOCG*` ioctls.
    pub const DROPPED: u16 = 3;
}

/// `EV_REL` codes.
pub mod rel {
    pub const X: u16 = 0x00;
    pub const Y: u16 = 0x01;
    pub const Z: u16 = 0x02;
    pub const RX: u16 = 0x03;
    pub const RY: u16 = 0x04;
    pub const RZ: u16 = 0x05;
    /// The horizontal wheel.
    pub const HWHEEL: u16 = 0x06;
    pub const DIAL: u16 = 0x07;
    /// The vertical wheel, in detents.
    pub const WHEEL: u16 = 0x08;
    pub const MISC: u16 = 0x09;
    pub const WHEEL_HI_RES: u16 = 0x0b;
    pub const HWHEEL_HI_RES: u16 = 0x0c;
    pub const MAX: u16 = 0x0f;
    pub const CNT: usize = MAX as usize + 1;
}

/// `EV_ABS` codes.
pub mod abs {
    pub const X: u16 = 0x00;
    pub const Y: u16 = 0x01;
    pub const Z: u16 = 0x02;
    pub const WHEEL: u16 = 0x08;
    pub const PRESSURE: u16 = 0x18;
    pub const MAX: u16 = 0x3f;
    pub const CNT: usize = MAX as usize + 1;
}

/// `EV_MSC` codes.
pub mod msc {
    pub const SERIAL: u16 = 0x00;
    pub const PULSELED: u16 = 0x01;
    pub const GESTURE: u16 = 0x02;
    pub const RAW: u16 = 0x03;
    /// The scancode of the key event that follows.
    pub const SCAN: u16 = 0x04;
    pub const TIMESTAMP: u16 = 0x05;
    pub const MAX: u16 = 0x07;
    pub const CNT: usize = MAX as usize + 1;
}

/// `EV_LED` codes.
pub mod led {
    pub const NUML: u16 = 0x00;
    pub const CAPSL: u16 = 0x01;
    pub const SCROLLL: u16 = 0x02;
    pub const COMPOSE: u16 = 0x03;
    pub const KANA: u16 = 0x04;
    pub const MAX: u16 = 0x0f;
    pub const CNT: usize = MAX as usize + 1;
}

/// `EV_REP` codes: the delay before the first repeat and the period
/// between repeats, in milliseconds.
pub mod rep {
    pub const DELAY: u16 = 0x00;
    pub const PERIOD: u16 = 0x01;
    pub const MAX: u16 = 0x01;
    pub const CNT: usize = MAX as usize + 1;
}

/// `EV_SW`, `EV_SND` and `EV_FF` limits (Poly's devices have none).
pub mod other {
    pub const SW_MAX: u16 = 0x10;
    pub const SND_MAX: u16 = 0x07;
    pub const FF_MAX: u16 = 0x7f;
}

/// Device properties (`INPUT_PROP_*`, `EVIOCGPROP`).
pub mod prop {
    /// Needs a pointer on screen (a touchpad, a mouse).
    pub const POINTER: u16 = 0x00;
    /// Directly mapped to the screen (a tablet, a touch screen).
    pub const DIRECT: u16 = 0x01;
    pub const BUTTONPAD: u16 = 0x02;
    pub const MAX: u16 = 0x1f;
    pub const CNT: usize = MAX as usize + 1;
}

/// Bus types (`BUS_*`, `struct input_id.bustype`, from `linux/input.h`).
pub mod bus {
    pub const PCI: u16 = 0x01;
    pub const USB: u16 = 0x03;
    pub const VIRTUAL: u16 = 0x06;
    pub const I8042: u16 = 0x11;
    pub const HOST: u16 = 0x19;
}

/// The highest key code (`KEY_MAX`); key bitmaps have `KEY_CNT` bits.
pub const KEY_MAX: u16 = 0x2ff;
pub const KEY_CNT: usize = KEY_MAX as usize + 1;

/// Key codes (`KEY_*`) of the keys [`crate::Key`] names, and the mouse
/// buttons (`BTN_*`).
pub mod key {
    pub const RESERVED: u16 = 0;
    pub const ESC: u16 = 1;
    pub const N1: u16 = 2;
    pub const N2: u16 = 3;
    pub const N3: u16 = 4;
    pub const N4: u16 = 5;
    pub const N5: u16 = 6;
    pub const N6: u16 = 7;
    pub const N7: u16 = 8;
    pub const N8: u16 = 9;
    pub const N9: u16 = 10;
    pub const N0: u16 = 11;
    pub const MINUS: u16 = 12;
    pub const EQUAL: u16 = 13;
    pub const BACKSPACE: u16 = 14;
    pub const TAB: u16 = 15;
    pub const Q: u16 = 16;
    pub const W: u16 = 17;
    pub const E: u16 = 18;
    pub const R: u16 = 19;
    pub const T: u16 = 20;
    pub const Y: u16 = 21;
    pub const U: u16 = 22;
    pub const I: u16 = 23;
    pub const O: u16 = 24;
    pub const P: u16 = 25;
    pub const LEFTBRACE: u16 = 26;
    pub const RIGHTBRACE: u16 = 27;
    pub const ENTER: u16 = 28;
    pub const LEFTCTRL: u16 = 29;
    pub const A: u16 = 30;
    pub const S: u16 = 31;
    pub const D: u16 = 32;
    pub const F: u16 = 33;
    pub const G: u16 = 34;
    pub const H: u16 = 35;
    pub const J: u16 = 36;
    pub const K: u16 = 37;
    pub const L: u16 = 38;
    pub const SEMICOLON: u16 = 39;
    pub const APOSTROPHE: u16 = 40;
    pub const GRAVE: u16 = 41;
    pub const LEFTSHIFT: u16 = 42;
    pub const BACKSLASH: u16 = 43;
    pub const Z: u16 = 44;
    pub const X: u16 = 45;
    pub const C: u16 = 46;
    pub const V: u16 = 47;
    pub const B: u16 = 48;
    pub const N: u16 = 49;
    pub const M: u16 = 50;
    pub const COMMA: u16 = 51;
    pub const DOT: u16 = 52;
    pub const SLASH: u16 = 53;
    pub const RIGHTSHIFT: u16 = 54;
    pub const KPASTERISK: u16 = 55;
    pub const LEFTALT: u16 = 56;
    pub const SPACE: u16 = 57;
    pub const CAPSLOCK: u16 = 58;
    pub const F1: u16 = 59;
    pub const F2: u16 = 60;
    pub const F3: u16 = 61;
    pub const F4: u16 = 62;
    pub const F5: u16 = 63;
    pub const F6: u16 = 64;
    pub const F7: u16 = 65;
    pub const F8: u16 = 66;
    pub const F9: u16 = 67;
    pub const F10: u16 = 68;
    pub const NUMLOCK: u16 = 69;
    pub const SCROLLLOCK: u16 = 70;
    pub const KP7: u16 = 71;
    pub const KP8: u16 = 72;
    pub const KP9: u16 = 73;
    pub const KPMINUS: u16 = 74;
    pub const KP4: u16 = 75;
    pub const KP5: u16 = 76;
    pub const KP6: u16 = 77;
    pub const KPPLUS: u16 = 78;
    pub const KP1: u16 = 79;
    pub const KP2: u16 = 80;
    pub const KP3: u16 = 81;
    pub const KP0: u16 = 82;
    pub const KPDOT: u16 = 83;
    /// The ISO key between the left Shift and Z.
    pub const KEY_102ND: u16 = 86;
    pub const F11: u16 = 87;
    pub const F12: u16 = 88;
    pub const KPENTER: u16 = 96;
    pub const RIGHTCTRL: u16 = 97;
    pub const KPSLASH: u16 = 98;
    pub const SYSRQ: u16 = 99;
    pub const RIGHTALT: u16 = 100;
    pub const HOME: u16 = 102;
    pub const UP: u16 = 103;
    pub const PAGEUP: u16 = 104;
    pub const LEFT: u16 = 105;
    pub const RIGHT: u16 = 106;
    pub const END: u16 = 107;
    pub const DOWN: u16 = 108;
    pub const PAGEDOWN: u16 = 109;
    pub const INSERT: u16 = 110;
    pub const DELETE: u16 = 111;
    pub const PAUSE: u16 = 119;
    pub const LEFTMETA: u16 = 125;
    pub const RIGHTMETA: u16 = 126;
    /// The Application (context menu) key on PC keyboards.
    pub const COMPOSE: u16 = 127;
    pub const MENU: u16 = 139;

    /// The first button code; codes below it are keyboard keys.
    pub const BTN_MISC: u16 = 0x100;
    pub const BTN_MOUSE: u16 = 0x110;
    pub const BTN_LEFT: u16 = 0x110;
    pub const BTN_RIGHT: u16 = 0x111;
    pub const BTN_MIDDLE: u16 = 0x112;
    pub const BTN_SIDE: u16 = 0x113;
    pub const BTN_EXTRA: u16 = 0x114;
    pub const BTN_FORWARD: u16 = 0x115;
    pub const BTN_BACK: u16 = 0x116;
    pub const BTN_TASK: u16 = 0x117;
    pub const BTN_TOOL_PEN: u16 = 0x140;
    pub const BTN_TOUCH: u16 = 0x14a;
}
