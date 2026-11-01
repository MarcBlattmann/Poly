//! What the server sends after the setup ("X Window System Protocol",
//! appendix B, "Errors" and "Events"): 32-byte errors (first byte 0),
//! replies (first byte 1, 32 bytes plus their length field in 4-byte
//! units) and events (codes 2 to 34; the top bit set when `SendEvent`
//! made them; `GenericEvent`, 35, carries a length like a reply).
//!
//! Every message but `KeymapNotify` has the low 16 bits of the sequence
//! number of the last request the server handled at bytes 2..4.

use crate::Xid;
use crate::wire::{Writer, u8_at, u16_at, u32_at};
use alloc::vec::Vec;

/// Event codes.
pub mod code {
    pub const KEY_PRESS: u8 = 2;
    pub const KEY_RELEASE: u8 = 3;
    pub const FOCUS_IN: u8 = 9;
    pub const FOCUS_OUT: u8 = 10;
    pub const KEYMAP_NOTIFY: u8 = 11;
    pub const EXPOSE: u8 = 12;
    pub const CREATE_NOTIFY: u8 = 16;
    pub const DESTROY_NOTIFY: u8 = 17;
    pub const UNMAP_NOTIFY: u8 = 18;
    pub const MAP_NOTIFY: u8 = 19;
    pub const MAP_REQUEST: u8 = 20;
    pub const REPARENT_NOTIFY: u8 = 21;
    pub const CONFIGURE_NOTIFY: u8 = 22;
    pub const CONFIGURE_REQUEST: u8 = 23;
    pub const PROPERTY_NOTIFY: u8 = 28;
    pub const SELECTION_CLEAR: u8 = 29;
    pub const SELECTION_REQUEST: u8 = 30;
    pub const SELECTION_NOTIFY: u8 = 31;
    pub const CLIENT_MESSAGE: u8 = 33;
    pub const MAPPING_NOTIFY: u8 = 34;
    pub const GENERIC_EVENT: u8 = 35;
}

/// Error codes.
pub mod error_code {
    pub const WINDOW: u8 = 3;
    pub const ATOM: u8 = 5;
    pub const MATCH: u8 = 8;
    pub const ACCESS: u8 = 10;
}

/// An X error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XError {
    pub code: u8,
    pub sequence: u16,
    /// The bad resource ID or value.
    pub value: u32,
    pub minor_opcode: u16,
    pub major_opcode: u8,
}

/// An event the window manager handles; `Other` keeps the code of the
/// rest. `send_event` is whether a client made it with `SendEvent`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    KeyPress {
        detail: u8,
        window: Xid,
    },
    FocusIn {
        detail: u8,
        window: Xid,
        mode: u8,
    },
    FocusOut {
        detail: u8,
        window: Xid,
        mode: u8,
    },
    CreateNotify {
        parent: Xid,
        window: Xid,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        override_redirect: bool,
    },
    DestroyNotify {
        event: Xid,
        window: Xid,
    },
    UnmapNotify {
        event: Xid,
        window: Xid,
        from_configure: bool,
    },
    MapNotify {
        event: Xid,
        window: Xid,
        override_redirect: bool,
    },
    MapRequest {
        parent: Xid,
        window: Xid,
    },
    ReparentNotify {
        event: Xid,
        window: Xid,
        parent: Xid,
        x: i16,
        y: i16,
        override_redirect: bool,
    },
    ConfigureNotify {
        event: Xid,
        window: Xid,
        above_sibling: Xid,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        override_redirect: bool,
    },
    ConfigureRequest {
        stack_mode: u8,
        parent: Xid,
        window: Xid,
        sibling: Xid,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        value_mask: u16,
    },
    PropertyNotify {
        window: Xid,
        atom: Xid,
        time: u32,
        /// 0 `NewValue`, 1 `Deleted`.
        state: u8,
    },
    SelectionClear {
        time: u32,
        owner: Xid,
        selection: Xid,
    },
    SelectionRequest {
        time: u32,
        owner: Xid,
        requestor: Xid,
        selection: Xid,
        target: Xid,
        property: Xid,
    },
    SelectionNotify {
        time: u32,
        requestor: Xid,
        selection: Xid,
        target: Xid,
        property: Xid,
    },
    ClientMessage {
        format: u8,
        window: Xid,
        ty: Xid,
        data: [u8; 20],
    },
    Other {
        code: u8,
    },
}

impl Event {
    /// Decode a 32-byte event (the top bit of the code is masked off).
    pub fn decode(b: &[u8; 32]) -> Event {
        let flag = |at: usize| u8_at(b, at) != 0;
        match b[0] & 0x7f {
            code::KEY_PRESS => Event::KeyPress { detail: b[1], window: u32_at(b, 12) },
            code::FOCUS_IN => Event::FocusIn { detail: b[1], window: u32_at(b, 4), mode: b[8] },
            code::FOCUS_OUT => Event::FocusOut { detail: b[1], window: u32_at(b, 4), mode: b[8] },
            code::CREATE_NOTIFY => Event::CreateNotify {
                parent: u32_at(b, 4),
                window: u32_at(b, 8),
                x: u16_at(b, 12) as i16,
                y: u16_at(b, 14) as i16,
                width: u16_at(b, 16),
                height: u16_at(b, 18),
                border_width: u16_at(b, 20),
                override_redirect: flag(22),
            },
            code::DESTROY_NOTIFY => Event::DestroyNotify { event: u32_at(b, 4), window: u32_at(b, 8) },
            code::UNMAP_NOTIFY => {
                Event::UnmapNotify { event: u32_at(b, 4), window: u32_at(b, 8), from_configure: flag(12) }
            }
            code::MAP_NOTIFY => {
                Event::MapNotify { event: u32_at(b, 4), window: u32_at(b, 8), override_redirect: flag(12) }
            }
            code::MAP_REQUEST => Event::MapRequest { parent: u32_at(b, 4), window: u32_at(b, 8) },
            code::REPARENT_NOTIFY => Event::ReparentNotify {
                event: u32_at(b, 4),
                window: u32_at(b, 8),
                parent: u32_at(b, 12),
                x: u16_at(b, 16) as i16,
                y: u16_at(b, 18) as i16,
                override_redirect: flag(20),
            },
            code::CONFIGURE_NOTIFY => Event::ConfigureNotify {
                event: u32_at(b, 4),
                window: u32_at(b, 8),
                above_sibling: u32_at(b, 12),
                x: u16_at(b, 16) as i16,
                y: u16_at(b, 18) as i16,
                width: u16_at(b, 20),
                height: u16_at(b, 22),
                border_width: u16_at(b, 24),
                override_redirect: flag(26),
            },
            code::CONFIGURE_REQUEST => Event::ConfigureRequest {
                stack_mode: b[1],
                parent: u32_at(b, 4),
                window: u32_at(b, 8),
                sibling: u32_at(b, 12),
                x: u16_at(b, 16) as i16,
                y: u16_at(b, 18) as i16,
                width: u16_at(b, 20),
                height: u16_at(b, 22),
                border_width: u16_at(b, 24),
                value_mask: u16_at(b, 26),
            },
            code::PROPERTY_NOTIFY => {
                Event::PropertyNotify { window: u32_at(b, 4), atom: u32_at(b, 8), time: u32_at(b, 12), state: b[16] }
            }
            code::SELECTION_CLEAR => {
                Event::SelectionClear { time: u32_at(b, 4), owner: u32_at(b, 8), selection: u32_at(b, 12) }
            }
            code::SELECTION_REQUEST => Event::SelectionRequest {
                time: u32_at(b, 4),
                owner: u32_at(b, 8),
                requestor: u32_at(b, 12),
                selection: u32_at(b, 16),
                target: u32_at(b, 20),
                property: u32_at(b, 24),
            },
            code::SELECTION_NOTIFY => Event::SelectionNotify {
                time: u32_at(b, 4),
                requestor: u32_at(b, 8),
                selection: u32_at(b, 12),
                target: u32_at(b, 16),
                property: u32_at(b, 20),
            },
            code::CLIENT_MESSAGE => {
                let mut data = [0; 20];
                data.copy_from_slice(&b[12..32]);
                Event::ClientMessage { format: b[1], window: u32_at(b, 4), ty: u32_at(b, 8), data }
            }
            code => Event::Other { code },
        }
    }

    /// The event's 32 bytes with sequence number `sequence` (what a
    /// server sends, or what `SendEvent` carries with any sequence).
    pub fn encode(&self, sequence: u16) -> [u8; 32] {
        let mut v = Vec::with_capacity(32);
        let mut w = Writer(&mut v);
        let b = u8::from;
        match self {
            Event::KeyPress { detail, window } => {
                w.u8(code::KEY_PRESS).u8(*detail).u16(sequence).u32(0).u32(0).u32(*window);
            }
            Event::FocusIn { detail, window, mode } | Event::FocusOut { detail, window, mode } => {
                let c = if matches!(self, Event::FocusIn { .. }) { code::FOCUS_IN } else { code::FOCUS_OUT };
                w.u8(c).u8(*detail).u16(sequence).u32(*window).u8(*mode);
            }
            Event::CreateNotify { parent, window, x, y, width, height, border_width, override_redirect } => {
                w.u8(code::CREATE_NOTIFY).u8(0).u16(sequence).u32(*parent).u32(*window).i16(*x).i16(*y);
                w.u16(*width).u16(*height).u16(*border_width).u8(b(*override_redirect));
            }
            Event::DestroyNotify { event, window } => {
                w.u8(code::DESTROY_NOTIFY).u8(0).u16(sequence).u32(*event).u32(*window);
            }
            Event::UnmapNotify { event, window, from_configure } => {
                w.u8(code::UNMAP_NOTIFY).u8(0).u16(sequence).u32(*event).u32(*window).u8(b(*from_configure));
            }
            Event::MapNotify { event, window, override_redirect } => {
                w.u8(code::MAP_NOTIFY).u8(0).u16(sequence).u32(*event).u32(*window).u8(b(*override_redirect));
            }
            Event::MapRequest { parent, window } => {
                w.u8(code::MAP_REQUEST).u8(0).u16(sequence).u32(*parent).u32(*window);
            }
            Event::ReparentNotify { event, window, parent, x, y, override_redirect } => {
                w.u8(code::REPARENT_NOTIFY).u8(0).u16(sequence).u32(*event).u32(*window).u32(*parent);
                w.i16(*x).i16(*y).u8(b(*override_redirect));
            }
            Event::ConfigureNotify {
                event,
                window,
                above_sibling,
                x,
                y,
                width,
                height,
                border_width,
                override_redirect,
            } => {
                w.u8(code::CONFIGURE_NOTIFY).u8(0).u16(sequence).u32(*event).u32(*window).u32(*above_sibling);
                w.i16(*x).i16(*y).u16(*width).u16(*height).u16(*border_width).u8(b(*override_redirect));
            }
            Event::ConfigureRequest {
                stack_mode,
                parent,
                window,
                sibling,
                x,
                y,
                width,
                height,
                border_width,
                value_mask,
            } => {
                w.u8(code::CONFIGURE_REQUEST).u8(*stack_mode).u16(sequence).u32(*parent).u32(*window).u32(*sibling);
                w.i16(*x).i16(*y).u16(*width).u16(*height).u16(*border_width).u16(*value_mask);
            }
            Event::PropertyNotify { window, atom, time, state } => {
                w.u8(code::PROPERTY_NOTIFY).u8(0).u16(sequence).u32(*window).u32(*atom).u32(*time).u8(*state);
            }
            Event::SelectionClear { time, owner, selection } => {
                w.u8(code::SELECTION_CLEAR).u8(0).u16(sequence).u32(*time).u32(*owner).u32(*selection);
            }
            Event::SelectionRequest { time, owner, requestor, selection, target, property } => {
                w.u8(code::SELECTION_REQUEST).u8(0).u16(sequence).u32(*time).u32(*owner).u32(*requestor);
                w.u32(*selection).u32(*target).u32(*property);
            }
            Event::SelectionNotify { time, requestor, selection, target, property } => {
                w.u8(code::SELECTION_NOTIFY).u8(0).u16(sequence).u32(*time).u32(*requestor).u32(*selection);
                w.u32(*target).u32(*property);
            }
            Event::ClientMessage { format, window, ty, data } => {
                w.u8(code::CLIENT_MESSAGE).u8(*format).u16(sequence).u32(*window).u32(*ty).bytes(data);
            }
            Event::Other { code } => {
                w.u8(*code).u8(0).u16(sequence);
            }
        }
        v.resize(32, 0);
        let mut out = [0; 32];
        out.copy_from_slice(&v);
        out
    }
}

/// A `ClientMessage` of format 32 with five 32-bit items.
pub fn client_message32(window: Xid, ty: Xid, items: [u32; 5]) -> Event {
    let mut data = [0; 20];
    for (i, v) in items.iter().enumerate() {
        data[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    Event::ClientMessage { format: 32, window, ty, data }
}

/// The five 32-bit items of a `ClientMessage`'s data.
pub fn data32(data: &[u8; 20]) -> [u32; 5] {
    let mut out = [0; 5];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u32_at(data, i * 4);
    }
    out
}

/// One message from the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// A reply: its sequence number (low 16 bits) and all of its bytes.
    Reply {
        sequence: u16,
        bytes: Vec<u8>,
    },
    Error(XError),
    /// An event, and whether `SendEvent` made it.
    Event {
        event: Event,
        sequence: u16,
        send_event: bool,
    },
    /// A `GenericEvent` (extension events), skipped whole.
    Generic {
        sequence: u16,
    },
}

/// The next message at the start of `b`: `None` until all of it is there,
/// else the message and how many bytes it took.
pub fn next_message(b: &[u8]) -> Option<(Message, usize)> {
    if b.len() < 32 {
        return None;
    }
    let sequence = u16_at(b, 2);
    match b[0] & 0x7f {
        0 => Some((
            Message::Error(XError {
                code: b[1],
                sequence,
                value: u32_at(b, 4),
                minor_opcode: u16_at(b, 8),
                major_opcode: b[10],
            }),
            32,
        )),
        1 if b[0] == 1 => {
            let total = 32 + 4 * u32_at(b, 4) as usize;
            (b.len() >= total).then(|| (Message::Reply { sequence, bytes: b[..total].to_vec() }, total))
        }
        code::GENERIC_EVENT => {
            let total = 32 + 4 * u32_at(b, 4) as usize;
            (b.len() >= total).then_some((Message::Generic { sequence }, total))
        }
        _ => {
            let mut e = [0; 32];
            e.copy_from_slice(&b[..32]);
            Some((Message::Event { event: Event::decode(&e), sequence, send_event: b[0] & 0x80 != 0 }, 32))
        }
    }
}

/// An error as a server sends it (for tests).
pub fn encode_error(e: &XError) -> [u8; 32] {
    let mut v = Vec::with_capacity(32);
    Writer(&mut v).u8(0).u8(e.code).u16(e.sequence).u32(e.value).u16(e.minor_opcode).u8(e.major_opcode);
    v.resize(32, 0);
    let mut out = [0; 32];
    out.copy_from_slice(&v);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn samples() -> Vec<Event> {
        let mut data = [0; 20];
        data[0] = 0x2a;
        vec![
            Event::KeyPress { detail: 38, window: 0x40_0001 },
            Event::FocusIn { detail: 3, window: 9, mode: 0 },
            Event::FocusOut { detail: 3, window: 9, mode: 1 },
            Event::CreateNotify {
                parent: 0x3b6,
                window: 0x0040_0001,
                x: -5,
                y: 7,
                width: 150,
                height: 100,
                border_width: 1,
                override_redirect: true,
            },
            Event::DestroyNotify { event: 0x3b6, window: 9 },
            Event::UnmapNotify { event: 0x3b6, window: 9, from_configure: true },
            Event::MapNotify { event: 0x3b6, window: 9, override_redirect: false },
            Event::MapRequest { parent: 0x3b6, window: 9 },
            Event::ReparentNotify { event: 0x3b6, window: 9, parent: 8, x: 1, y: -2, override_redirect: true },
            Event::ConfigureNotify {
                event: 9,
                window: 9,
                above_sibling: 0,
                x: 10,
                y: -20,
                width: 300,
                height: 200,
                border_width: 0,
                override_redirect: false,
            },
            Event::ConfigureRequest {
                stack_mode: 0,
                parent: 0x3b6,
                window: 9,
                sibling: 0,
                x: 1,
                y: 2,
                width: 3,
                height: 4,
                border_width: 5,
                value_mask: 0x0f,
            },
            Event::PropertyNotify { window: 9, atom: 39, time: 1234, state: 1 },
            Event::SelectionClear { time: 1, owner: 2, selection: 3 },
            Event::SelectionRequest { time: 1, owner: 2, requestor: 3, selection: 4, target: 5, property: 6 },
            Event::SelectionNotify { time: 1, requestor: 2, selection: 3, target: 4, property: 0 },
            Event::ClientMessage { format: 32, window: 9, ty: 300, data },
        ]
    }

    #[test]
    fn events_round_trip_through_their_bytes() {
        for e in samples() {
            let b = e.encode(0x1234);
            assert_eq!(u16_at(&b, 2), 0x1234, "{e:?}");
            assert_eq!(Event::decode(&b), e);
            let (m, n) = next_message(&b).unwrap();
            assert_eq!(n, 32);
            assert_eq!(m, Message::Event { event: e, sequence: 0x1234, send_event: false });
        }
    }

    #[test]
    fn event_offsets_follow_the_protocol() {
        // ConfigureNotify: window at 8, x at 16, width at 20, override-redirect at 26.
        let b = samples()[9].encode(0);
        assert_eq!((b[0], u32_at(&b, 8), u16_at(&b, 16), u16_at(&b, 20), b[26]), (22, 9, 10, 300, 0));
        // ClientMessage: format at 1, window at 4, type at 8, data from 12.
        let m = client_message32(9, 300, [1, 2, 3, 4, 5]);
        let b = m.encode(0);
        assert_eq!((b[0], b[1], u32_at(&b, 4), u32_at(&b, 8), u32_at(&b, 12), u32_at(&b, 28)), (33, 32, 9, 300, 1, 5));
        let Event::ClientMessage { data, .. } = m else { unreachable!() };
        assert_eq!(data32(&data), [1, 2, 3, 4, 5]);
    }

    #[test]
    fn splits_replies_errors_and_events() {
        // A reply with two extra words, then an error, then a sent event.
        let mut b = crate::request::encode_reply(8, 7, &[0u8; 32]);
        assert_eq!(b.len(), 40);
        let err = XError { code: 3, sequence: 8, value: 0x99, minor_opcode: 0, major_opcode: 20 };
        b.extend_from_slice(&encode_error(&err));
        let mut sent = Event::MapRequest { parent: 1, window: 2 }.encode(9);
        sent[0] |= 0x80;
        b.extend_from_slice(&sent);
        let (m, n) = next_message(&b).unwrap();
        assert_eq!(n, 40);
        assert!(matches!(m, Message::Reply { sequence: 7, ref bytes } if bytes.len() == 40));
        assert_eq!(next_message(&b[40..72]), Some((Message::Error(err), 32)));
        let (m, _) = next_message(&b[72..]).unwrap();
        assert_eq!(
            m,
            Message::Event { event: Event::MapRequest { parent: 1, window: 2 }, sequence: 9, send_event: true }
        );
        // Incomplete: nothing yet.
        assert_eq!(next_message(&b[..31]), None);
        assert_eq!(next_message(&b[..39]), None);
        // A generic event with one extra word is skipped whole.
        let mut g = [0u8; 36];
        (g[0], g[4]) = (35, 1);
        assert_eq!(next_message(&g[..32]), None);
        assert_eq!(next_message(&g), Some((Message::Generic { sequence: 0 }, 36)));
        // Unknown codes stay events.
        let mut k = [0u8; 32];
        k[0] = 11;
        assert!(matches!(next_message(&k), Some((Message::Event { event: Event::Other { code: 11 }, .. }, 32))));
    }
}
