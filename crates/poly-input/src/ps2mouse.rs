//! The PS/2 mouse on the i8042's second (auxiliary) port: its commands,
//! the IntelliMouse wheel detection and its movement packets.
//!
//! Sources: IBM, "Personal System/2 Hardware Interface Technical
//! Reference" (the auxiliary device: commands, the 3-byte packet);
//! Microsoft, the IntelliMouse PS/2 protocol extensions as documented
//! publicly (the "magic" sample rate sequences 200, 100, 80, which turn
//! on the wheel and make the mouse report ID 3, and 200, 200, 80, which
//! turn on buttons 4 and 5 and make it report ID 4; their 4-byte packets);
//! the OSDev wiki pages "PS/2 Mouse" and "Mouse Input".
//!
//! # Packets
//!
//! ```text
//! byte 0: Y overflow | X overflow | Y sign | X sign | 1 | middle | right | left
//! byte 1: X movement (low 8 bits of a 9-bit two's complement number, sign in byte 0)
//! byte 2: Y movement (likewise; positive is up)
//! byte 3: ID 3: Z (wheel) movement, two's complement
//!         ID 4: bits 0-3 Z movement (4-bit two's complement), bit 4 button 4, bit 5 button 5
//! ```
//!
//! [`MouseDecoder`] takes bytes and returns packets. Bit 3 of the first
//! byte is always set, which is how it finds the start of a packet again
//! after a lost byte; a byte that arrives long after the one before it
//! starts a new packet too. [`MouseState`] turns packets into evdev events:
//! `REL_X`, `REL_Y` (positive is down, so the PS/2 Y is negated),
//! `REL_WHEEL` (positive is up: the wheel turned away from the user) and
//! the buttons' presses and releases.

use crate::codes::{ev, key, rel};

// Mouse commands (sent through the controller's "write to auxiliary
// device" command, 0xD4).
pub const SET_SCALING_1_1: u8 = 0xe6;
pub const SET_RESOLUTION: u8 = 0xe8;
pub const STATUS_REQUEST: u8 = 0xe9;
pub const GET_ID: u8 = 0xf2;
pub const SET_SAMPLE_RATE: u8 = 0xf3;
pub const ENABLE_REPORTING: u8 = 0xf4;
pub const DISABLE_REPORTING: u8 = 0xf5;
pub const SET_DEFAULTS: u8 = 0xf6;
pub const RESET: u8 = 0xff;

/// What the mouse reports to `GET_ID`.
pub const ID_STANDARD: u8 = 0x00;
pub const ID_WHEEL: u8 = 0x03;
pub const ID_FIVE_BUTTONS: u8 = 0x04;

/// The sample rates that turn on the wheel, then buttons 4 and 5.
pub const WHEEL_KNOCK: [u8; 3] = [200, 100, 80];
pub const FIVE_BUTTON_KNOCK: [u8; 3] = [200, 200, 80];

/// Reports per second once set up.
pub const SAMPLE_RATE: u8 = 100;
/// Resolution code 3: 8 counts per millimetre.
pub const RESOLUTION: u8 = 3;

/// A byte arriving this long after the previous one starts a new packet
/// (a packet's bytes come within a few milliseconds of each other).
pub const PACKET_GAP_NS: u64 = 50_000_000;

/// Which protocol the mouse speaks, from its ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Three buttons, 3-byte packets.
    Standard,
    /// IntelliMouse: a wheel, 4-byte packets.
    Wheel,
    /// IntelliMouse Explorer: a wheel and five buttons, 4-byte packets.
    FiveButtons,
}

impl Kind {
    pub const fn from_id(id: u8) -> Kind {
        match id {
            ID_WHEEL => Kind::Wheel,
            ID_FIVE_BUTTONS => Kind::FiveButtons,
            _ => Kind::Standard,
        }
    }

    pub const fn packet_len(self) -> usize {
        match self {
            Kind::Standard => 3,
            Kind::Wheel | Kind::FiveButtons => 4,
        }
    }

    pub const fn has_wheel(self) -> bool {
        !matches!(self, Kind::Standard)
    }

    /// The evdev buttons this kind of mouse has.
    pub const fn buttons(self) -> &'static [u16] {
        match self {
            Kind::Standard | Kind::Wheel => &[key::BTN_LEFT, key::BTN_RIGHT, key::BTN_MIDDLE],
            Kind::FiveButtons => &[key::BTN_LEFT, key::BTN_RIGHT, key::BTN_MIDDLE, key::BTN_SIDE, key::BTN_EXTRA],
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Kind::Standard => "PS/2 Mouse",
            Kind::Wheel => "PS/2 IntelliMouse",
            Kind::FiveButtons => "PS/2 IntelliMouse Explorer",
        }
    }
}

/// Button bits of a [`Packet`].
pub mod button {
    pub const LEFT: u8 = 1 << 0;
    pub const RIGHT: u8 = 1 << 1;
    pub const MIDDLE: u8 = 1 << 2;
    pub const FOURTH: u8 = 1 << 3;
    pub const FIFTH: u8 = 1 << 4;
}

/// One movement report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Packet {
    /// Rightwards motion.
    pub dx: i32,
    /// Upwards motion (the PS/2 sense).
    pub dy: i32,
    /// Wheel detents, positive up (away from the user).
    pub wheel: i32,
    /// [`button`] bits.
    pub buttons: u8,
}

/// The first byte's bits.
const ALWAYS_ONE: u8 = 1 << 3;
const X_SIGN: u8 = 1 << 4;
const Y_SIGN: u8 = 1 << 5;
const X_OVERFLOW: u8 = 1 << 6;
const Y_OVERFLOW: u8 = 1 << 7;

/// Assembles packets from the mouse's bytes.
#[derive(Clone, Debug)]
pub struct MouseDecoder {
    kind: Kind,
    buf: [u8; 4],
    len: usize,
    last_ns: u64,
    /// Bytes thrown away to find the start of a packet.
    pub resyncs: u64,
}

impl MouseDecoder {
    pub const fn new(kind: Kind) -> MouseDecoder {
        MouseDecoder { kind, buf: [0; 4], len: 0, last_ns: 0, resyncs: 0 }
    }

    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// Forget a partial packet.
    pub fn reset(&mut self) {
        self.len = 0;
    }

    /// Feed one byte, which arrived at `now_ns`; returns the packet it
    /// completes.
    pub fn feed(&mut self, byte: u8, now_ns: u64) -> Option<Packet> {
        if self.len > 0 && now_ns.wrapping_sub(self.last_ns) > PACKET_GAP_NS {
            self.resyncs += self.len as u64;
            self.len = 0;
        }
        self.last_ns = now_ns;
        if self.len == 0 && byte & ALWAYS_ONE == 0 {
            self.resyncs += 1;
            return None;
        }
        self.buf[self.len] = byte;
        self.len += 1;
        if self.len < self.kind.packet_len() {
            return None;
        }
        self.len = 0;
        Some(self.decode())
    }

    fn decode(&self) -> Packet {
        let [b0, b1, b2, b3] = self.buf;
        // A 9-bit two's complement value; on overflow the count is
        // meaningless, so it is dropped.
        let axis = |low: u8, sign: bool, overflow: bool| {
            if overflow { 0 } else { i32::from(low) - if sign { 256 } else { 0 } }
        };
        let dx = axis(b1, b0 & X_SIGN != 0, b0 & X_OVERFLOW != 0);
        let dy = axis(b2, b0 & Y_SIGN != 0, b0 & Y_OVERFLOW != 0);
        let mut buttons = b0 & (button::LEFT | button::RIGHT | button::MIDDLE);
        // The mouse counts Z positive when the wheel turns towards the
        // user (scrolling down).
        let wheel = match self.kind {
            Kind::Standard => 0,
            Kind::Wheel => -i32::from(b3 as i8),
            Kind::FiveButtons => {
                if b3 & 0x10 != 0 {
                    buttons |= button::FOURTH;
                }
                if b3 & 0x20 != 0 {
                    buttons |= button::FIFTH;
                }
                // Bits 0-3 as a signed 4-bit number.
                -i32::from(((b3 << 4) as i8) >> 4)
            }
        };
        Packet { dx, dy, wheel, buttons }
    }
}

/// The buttons held, to report changes as presses and releases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseState {
    buttons: u8,
}

/// Events for one packet: at most motion, wheel, five buttons and the
/// `SYN_REPORT`.
pub type PacketEvents = ([(u16, u16, i32); 9], usize);

impl MouseState {
    pub const fn new() -> MouseState {
        MouseState { buttons: 0 }
    }

    /// The evdev events for `p`, ending with `SYN_REPORT`.
    pub fn events(&mut self, p: &Packet) -> PacketEvents {
        let mut out = [(0u16, 0u16, 0i32); 9];
        let mut n = 0;
        let mut push = |e| {
            out[n] = e;
            n += 1;
        };
        if p.dx != 0 {
            push((ev::REL, rel::X, p.dx));
        }
        if p.dy != 0 {
            push((ev::REL, rel::Y, -p.dy));
        }
        if p.wheel != 0 {
            push((ev::REL, rel::WHEEL, p.wheel));
        }
        let changed = self.buttons ^ p.buttons;
        for (bit, code) in [
            (button::LEFT, key::BTN_LEFT),
            (button::RIGHT, key::BTN_RIGHT),
            (button::MIDDLE, key::BTN_MIDDLE),
            (button::FOURTH, key::BTN_SIDE),
            (button::FIFTH, key::BTN_EXTRA),
        ] {
            if changed & bit != 0 {
                push((ev::KEY, code, i32::from(p.buttons & bit != 0)));
            }
        }
        self.buttons = p.buttons;
        push((ev::SYN, crate::codes::syn::REPORT, 0));
        (out, n)
    }
}

/// The bytes of a packet with this motion and these buttons (for tests
/// and for faking mouse input through the controller): `dy` up is
/// positive, `wheel` up is positive. Motion is clamped to what a packet
/// can carry.
pub fn encode(kind: Kind, p: &Packet) -> ([u8; 4], usize) {
    let dx = p.dx.clamp(-256, 255);
    let dy = p.dy.clamp(-256, 255);
    let mut b0 = ALWAYS_ONE | (p.buttons & 0x07);
    if dx < 0 {
        b0 |= X_SIGN;
    }
    if dy < 0 {
        b0 |= Y_SIGN;
    }
    let b3 = match kind {
        Kind::Standard => 0,
        Kind::Wheel => (-p.wheel.clamp(-127, 127)) as i8 as u8,
        Kind::FiveButtons => {
            let z = ((-p.wheel.clamp(-7, 7)) as i8 as u8) & 0x0f;
            z | if p.buttons & button::FOURTH != 0 { 0x10 } else { 0 }
                | if p.buttons & button::FIFTH != 0 { 0x20 } else { 0 }
        }
    };
    ([b0, dx as u8, dy as u8, b3], kind.packet_len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codes::syn;
    use std::vec::Vec;

    fn decode_all(kind: Kind, bytes: &[u8]) -> Vec<Packet> {
        let mut d = MouseDecoder::new(kind);
        bytes.iter().filter_map(|&b| d.feed(b, 0)).collect()
    }

    #[test]
    fn standard_packets() {
        // Left button, 5 right, 3 up.
        assert_eq!(decode_all(Kind::Standard, &[0x09, 5, 3]), [Packet { dx: 5, dy: 3, wheel: 0, buttons: 1 }]);
        // Negative motion: sign bits with the low bytes.
        assert_eq!(decode_all(Kind::Standard, &[0x38, 0xfb, 0xff]), [Packet { dx: -5, dy: -1, wheel: 0, buttons: 0 }]);
        // Overflow drops the count.
        assert_eq!(decode_all(Kind::Standard, &[0x48, 0x12, 1]), [Packet { dx: 0, dy: 1, wheel: 0, buttons: 0 }]);
    }

    #[test]
    fn wheel_and_five_button_packets() {
        // Z = -1: the wheel turned away from the user, which is up.
        assert_eq!(decode_all(Kind::Wheel, &[0x08, 0, 0, 0xff]), [Packet { dx: 0, dy: 0, wheel: 1, buttons: 0 }]);
        assert_eq!(decode_all(Kind::Wheel, &[0x08, 0, 0, 0x02])[0].wheel, -2);
        // ID 4: Z in four bits, buttons 4 and 5 above it.
        let p = decode_all(Kind::FiveButtons, &[0x0c, 0, 0, 0x3f]);
        assert_eq!(p, [Packet { dx: 0, dy: 0, wheel: 1, buttons: button::MIDDLE | button::FOURTH | button::FIFTH }]);
        assert_eq!(decode_all(Kind::FiveButtons, &[0x08, 0, 0, 0x07])[0].wheel, -7);
    }

    #[test]
    fn resynchronises_on_bit_3_and_gaps() {
        let mut d = MouseDecoder::new(Kind::Standard);
        // A stray byte without bit 3 where a packet should start.
        assert_eq!(d.feed(0x05, 0), None);
        assert_eq!(d.resyncs, 1);
        assert_eq!(d.feed(0x08, 0), None);
        assert_eq!(d.feed(1, 0), None);
        // The packet's last byte is lost: the next byte, much later,
        // starts a new packet.
        assert_eq!(d.feed(0x09, PACKET_GAP_NS + 1), None);
        assert_eq!(d.feed(2, PACKET_GAP_NS + 2), None);
        assert_eq!(d.feed(0, PACKET_GAP_NS + 3), Some(Packet { dx: 2, dy: 0, wheel: 0, buttons: 1 }));
        assert_eq!(d.resyncs, 3);
        d.feed(0x08, 0);
        d.reset();
        assert_eq!(d.feed(0x08, 0), None);
    }

    #[test]
    fn packets_become_events() {
        let mut s = MouseState::new();
        let (e, n) = s.events(&Packet { dx: 3, dy: 2, wheel: -1, buttons: button::LEFT });
        assert_eq!(
            &e[..n],
            [
                (ev::REL, rel::X, 3),
                (ev::REL, rel::Y, -2),
                (ev::REL, rel::WHEEL, -1),
                (ev::KEY, key::BTN_LEFT, 1),
                (ev::SYN, syn::REPORT, 0)
            ]
        );
        // Only changes of buttons are reported.
        let (e, n) = s.events(&Packet { dx: 0, dy: 0, wheel: 0, buttons: button::LEFT | button::FIFTH });
        assert_eq!(&e[..n], [(ev::KEY, key::BTN_EXTRA, 1), (ev::SYN, syn::REPORT, 0)]);
        let (e, n) = s.events(&Packet::default());
        assert_eq!(&e[..n], [(ev::KEY, key::BTN_LEFT, 0), (ev::KEY, key::BTN_EXTRA, 0), (ev::SYN, syn::REPORT, 0)]);
        // Everything at once fits.
        let (_, n) = MouseState::new().events(&Packet { dx: 1, dy: 1, wheel: 1, buttons: 0x1f });
        assert_eq!(n, 9);
    }

    #[test]
    fn encode_round_trips() {
        for kind in [Kind::Standard, Kind::Wheel, Kind::FiveButtons] {
            for &(dx, dy, wheel, buttons) in
                &[(0, 0, 0, 0u8), (255, -256, 1, 7), (-1, 1, -1, 1), (17, -42, 3, 2), (-200, 100, -7, 0x1f)]
            {
                let buttons = if kind == Kind::FiveButtons { buttons } else { buttons & 7 };
                let wheel = if kind == Kind::Standard { 0 } else { wheel };
                let p = Packet { dx, dy, wheel, buttons };
                let (b, n) = encode(kind, &p);
                assert_eq!(decode_all(kind, &b[..n]), [p], "{kind:?} {p:?}");
            }
        }
        assert_eq!(Kind::from_id(3), Kind::Wheel);
        assert_eq!(Kind::from_id(4), Kind::FiveButtons);
        assert_eq!(Kind::from_id(0), Kind::Standard);
        assert_eq!(Kind::FiveButtons.buttons().len(), 5);
    }
}
