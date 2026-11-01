//! Core requests ("X Window System Protocol", appendix B, "Requests").
//!
//! Every request starts with its major opcode, one byte of data (a small
//! argument or unused), and its length in 4-byte units, the header
//! included. Requests with a value list (`CreateWindow`,
//! `ChangeWindowAttributes`, `ConfigureWindow`) carry a mask and one
//! 4-byte value per bit set, in the order of the bits.

use crate::Xid;
use crate::wire::{Writer, pad, u8_at, u16_at, u32_at};
use alloc::vec::Vec;

/// Major opcodes.
pub mod opcode {
    pub const CREATE_WINDOW: u8 = 1;
    pub const CHANGE_WINDOW_ATTRIBUTES: u8 = 2;
    pub const GET_WINDOW_ATTRIBUTES: u8 = 3;
    pub const DESTROY_WINDOW: u8 = 4;
    pub const CHANGE_SAVE_SET: u8 = 6;
    pub const MAP_WINDOW: u8 = 8;
    pub const UNMAP_WINDOW: u8 = 10;
    pub const CONFIGURE_WINDOW: u8 = 12;
    pub const GET_GEOMETRY: u8 = 14;
    pub const INTERN_ATOM: u8 = 16;
    pub const CHANGE_PROPERTY: u8 = 18;
    pub const DELETE_PROPERTY: u8 = 19;
    pub const GET_PROPERTY: u8 = 20;
    pub const SET_SELECTION_OWNER: u8 = 22;
    pub const GET_SELECTION_OWNER: u8 = 23;
    pub const CONVERT_SELECTION: u8 = 24;
    pub const SEND_EVENT: u8 = 25;
    pub const SET_INPUT_FOCUS: u8 = 42;
    pub const GET_INPUT_FOCUS: u8 = 43;
    pub const QUERY_EXTENSION: u8 = 98;
    pub const KILL_CLIENT: u8 = 113;
}

/// Window attribute mask bits (`CreateWindow`, `ChangeWindowAttributes`).
pub mod cw {
    pub const BACK_PIXEL: u32 = 0x0002;
    pub const BORDER_PIXEL: u32 = 0x0008;
    pub const OVERRIDE_REDIRECT: u32 = 0x0200;
    pub const EVENT_MASK: u32 = 0x0800;
}

/// `ConfigureWindow` mask bits.
pub mod config {
    pub const X: u16 = 0x01;
    pub const Y: u16 = 0x02;
    pub const WIDTH: u16 = 0x04;
    pub const HEIGHT: u16 = 0x08;
    pub const BORDER_WIDTH: u16 = 0x10;
    pub const SIBLING: u16 = 0x20;
    pub const STACK_MODE: u16 = 0x40;
}

/// `ConfigureWindow` stack modes.
pub mod stack {
    pub const ABOVE: u32 = 0;
    pub const BELOW: u32 = 1;
}

/// Event mask bits.
pub mod event_mask {
    pub const KEY_PRESS: u32 = 0x0000_0001;
    pub const STRUCTURE_NOTIFY: u32 = 0x0002_0000;
    pub const SUBSTRUCTURE_NOTIFY: u32 = 0x0008_0000;
    pub const SUBSTRUCTURE_REDIRECT: u32 = 0x0010_0000;
    pub const FOCUS_CHANGE: u32 = 0x0020_0000;
    pub const PROPERTY_CHANGE: u32 = 0x0040_0000;
}

/// Window classes.
pub mod class {
    pub const COPY_FROM_PARENT: u16 = 0;
    pub const INPUT_OUTPUT: u16 = 1;
    pub const INPUT_ONLY: u16 = 2;
}

/// `ChangeProperty` modes.
pub mod prop_mode {
    pub const REPLACE: u8 = 0;
    pub const PREPEND: u8 = 1;
    pub const APPEND: u8 = 2;
}

/// `SetInputFocus` revert-to values.
pub mod revert {
    pub const NONE: u8 = 0;
    pub const POINTER_ROOT: u8 = 1;
    pub const PARENT: u8 = 2;
}

/// A request. Value lists are `(mask bit, value)` pairs in any order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request<'a> {
    CreateWindow {
        depth: u8,
        wid: Xid,
        parent: Xid,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        class: u16,
        visual: u32,
        values: &'a [(u32, u32)],
    },
    ChangeWindowAttributes {
        window: Xid,
        values: &'a [(u32, u32)],
    },
    GetWindowAttributes {
        window: Xid,
    },
    DestroyWindow {
        window: Xid,
    },
    /// `insert`: add to the save-set, else delete.
    ChangeSaveSet {
        insert: bool,
        window: Xid,
    },
    MapWindow {
        window: Xid,
    },
    UnmapWindow {
        window: Xid,
    },
    ConfigureWindow {
        window: Xid,
        values: &'a [(u16, u32)],
    },
    GetGeometry {
        drawable: Xid,
    },
    InternAtom {
        only_if_exists: bool,
        name: &'a [u8],
    },
    /// `data` is the value in bytes (its length a multiple of `format / 8`).
    ChangeProperty {
        mode: u8,
        window: Xid,
        property: Xid,
        ty: Xid,
        format: u8,
        data: &'a [u8],
    },
    DeleteProperty {
        window: Xid,
        property: Xid,
    },
    GetProperty {
        delete: bool,
        window: Xid,
        property: Xid,
        /// 0: any type.
        ty: Xid,
        long_offset: u32,
        long_length: u32,
    },
    SetSelectionOwner {
        owner: Xid,
        selection: Xid,
        time: u32,
    },
    GetSelectionOwner {
        selection: Xid,
    },
    ConvertSelection {
        requestor: Xid,
        selection: Xid,
        target: Xid,
        property: Xid,
        time: u32,
    },
    SendEvent {
        propagate: bool,
        destination: Xid,
        event_mask: u32,
        event: [u8; 32],
    },
    SetInputFocus {
        revert_to: u8,
        focus: Xid,
        time: u32,
    },
    GetInputFocus,
    QueryExtension {
        name: &'a [u8],
    },
    KillClient {
        resource: Xid,
    },
    /// An extension's request: its major opcode (from `QueryExtension`),
    /// the minor opcode in the data byte, and the fields after the header.
    Extension {
        major: u8,
        minor: u8,
        body: &'a [u8],
        reply: bool,
    },
}

/// The Composite extension ("Composite Extension", version 0.4): its
/// name, the requests the window manager makes, and their bodies.
pub mod composite {
    use crate::Xid;
    use alloc::vec::Vec;

    pub const NAME: &[u8] = b"Composite";
    pub const QUERY_VERSION: u8 = 0;
    pub const REDIRECT_SUBWINDOWS: u8 = 2;
    /// `update` of the redirect requests.
    pub const REDIRECT_MANUAL: u8 = 1;

    /// `CompositeQueryVersion`'s body: the version the client speaks.
    pub fn query_version(major: u32, minor: u32) -> Vec<u8> {
        [major.to_le_bytes(), minor.to_le_bytes()].concat()
    }

    /// `CompositeRedirectSubwindows`'s body.
    pub fn redirect_subwindows(window: Xid, update: u8) -> Vec<u8> {
        let mut v = window.to_le_bytes().to_vec();
        v.extend_from_slice(&[update, 0, 0, 0]);
        v
    }
}

/// The reply to `QueryExtension`: the major opcode if the extension is
/// there.
pub fn query_extension_reply(reply: &[u8]) -> Option<u8> {
    (u8_at(reply, 8) != 0).then(|| u8_at(reply, 9))
}

impl Request<'_> {
    pub fn opcode(&self) -> u8 {
        use opcode::*;
        match self {
            Request::CreateWindow { .. } => CREATE_WINDOW,
            Request::ChangeWindowAttributes { .. } => CHANGE_WINDOW_ATTRIBUTES,
            Request::GetWindowAttributes { .. } => GET_WINDOW_ATTRIBUTES,
            Request::DestroyWindow { .. } => DESTROY_WINDOW,
            Request::ChangeSaveSet { .. } => CHANGE_SAVE_SET,
            Request::MapWindow { .. } => MAP_WINDOW,
            Request::UnmapWindow { .. } => UNMAP_WINDOW,
            Request::ConfigureWindow { .. } => CONFIGURE_WINDOW,
            Request::GetGeometry { .. } => GET_GEOMETRY,
            Request::InternAtom { .. } => INTERN_ATOM,
            Request::ChangeProperty { .. } => CHANGE_PROPERTY,
            Request::DeleteProperty { .. } => DELETE_PROPERTY,
            Request::GetProperty { .. } => GET_PROPERTY,
            Request::SetSelectionOwner { .. } => SET_SELECTION_OWNER,
            Request::GetSelectionOwner { .. } => GET_SELECTION_OWNER,
            Request::ConvertSelection { .. } => CONVERT_SELECTION,
            Request::SendEvent { .. } => SEND_EVENT,
            Request::SetInputFocus { .. } => SET_INPUT_FOCUS,
            Request::GetInputFocus => GET_INPUT_FOCUS,
            Request::QueryExtension { .. } => QUERY_EXTENSION,
            Request::KillClient { .. } => KILL_CLIENT,
            Request::Extension { major, .. } => *major,
        }
    }

    /// Whether the server answers with a reply.
    pub fn has_reply(&self) -> bool {
        matches!(
            self,
            Request::GetWindowAttributes { .. }
                | Request::GetGeometry { .. }
                | Request::InternAtom { .. }
                | Request::GetProperty { .. }
                | Request::GetSelectionOwner { .. }
                | Request::GetInputFocus
                | Request::QueryExtension { .. }
                | Request::Extension { reply: true, .. }
        )
    }

    /// Append the request's bytes to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        let op = self.opcode();
        let mut w = Writer(out);
        // The header; the length is filled in at the end.
        match self {
            Request::CreateWindow { depth, wid, parent, x, y, width, height, border_width, class, visual, values } => {
                w.u8(op).u8(*depth).u16(0).u32(*wid).u32(*parent).i16(*x).i16(*y).u16(*width).u16(*height);
                w.u16(*border_width).u16(*class).u32(*visual);
                value_list32(&mut w, values);
            }
            Request::ChangeWindowAttributes { window, values } => {
                w.u8(op).u8(0).u16(0).u32(*window);
                value_list32(&mut w, values);
            }
            Request::GetWindowAttributes { window }
            | Request::DestroyWindow { window }
            | Request::MapWindow { window }
            | Request::UnmapWindow { window } => {
                w.u8(op).u8(0).u16(0).u32(*window);
            }
            Request::ChangeSaveSet { insert, window } => {
                w.u8(op).u8(if *insert { 0 } else { 1 }).u16(0).u32(*window);
            }
            Request::ConfigureWindow { window, values } => {
                let mut sorted: Vec<(u16, u32)> = values.to_vec();
                sorted.sort_by_key(|(bit, _)| *bit);
                let mask = sorted.iter().fold(0, |m, (bit, _)| m | bit);
                w.u8(op).u8(0).u16(0).u32(*window).u16(mask).u16(0);
                for (_, v) in sorted {
                    w.u32(v);
                }
            }
            Request::GetGeometry { drawable } => {
                w.u8(op).u8(0).u16(0).u32(*drawable);
            }
            Request::InternAtom { only_if_exists, name } => {
                w.u8(op).u8(u8::from(*only_if_exists)).u16(0).u16(name.len() as u16).u16(0).padded(name);
            }
            Request::ChangeProperty { mode, window, property, ty, format, data } => {
                let unit = usize::from((*format / 8).max(1));
                w.u8(op).u8(*mode).u16(0).u32(*window).u32(*property).u32(*ty).u8(*format).zeros(3);
                w.u32((data.len() / unit) as u32).padded(data);
            }
            Request::DeleteProperty { window, property } => {
                w.u8(op).u8(0).u16(0).u32(*window).u32(*property);
            }
            Request::GetProperty { delete, window, property, ty, long_offset, long_length } => {
                w.u8(op).u8(u8::from(*delete)).u16(0).u32(*window).u32(*property).u32(*ty);
                w.u32(*long_offset).u32(*long_length);
            }
            Request::SetSelectionOwner { owner, selection, time } => {
                w.u8(op).u8(0).u16(0).u32(*owner).u32(*selection).u32(*time);
            }
            Request::GetSelectionOwner { selection } => {
                w.u8(op).u8(0).u16(0).u32(*selection);
            }
            Request::ConvertSelection { requestor, selection, target, property, time } => {
                w.u8(op).u8(0).u16(0).u32(*requestor).u32(*selection).u32(*target).u32(*property).u32(*time);
            }
            Request::SendEvent { propagate, destination, event_mask, event } => {
                w.u8(op).u8(u8::from(*propagate)).u16(0).u32(*destination).u32(*event_mask).bytes(event);
            }
            Request::SetInputFocus { revert_to, focus, time } => {
                w.u8(op).u8(*revert_to).u16(0).u32(*focus).u32(*time);
            }
            Request::GetInputFocus => {
                w.u8(op).u8(0).u16(0);
            }
            Request::QueryExtension { name } => {
                w.u8(op).u8(0).u16(0).u16(name.len() as u16).u16(0).padded(name);
            }
            Request::KillClient { resource } => {
                w.u8(op).u8(0).u16(0).u32(*resource);
            }
            Request::Extension { minor, body, .. } => {
                w.u8(op).u8(*minor).u16(0).padded(body);
            }
        }
        let units = ((out.len() - start) / 4) as u16;
        out[start + 2..start + 4].copy_from_slice(&units.to_le_bytes());
    }
}

/// A mask and its values, in the order of the bits.
fn value_list32(w: &mut Writer<'_>, values: &[(u32, u32)]) {
    let mut sorted: Vec<(u32, u32)> = values.to_vec();
    sorted.sort_by_key(|(bit, _)| *bit);
    w.u32(sorted.iter().fold(0, |m, (bit, _)| m | bit));
    for (_, v) in sorted {
        w.u32(v);
    }
}

/// The reply to `GetProperty`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropertyReply {
    /// 0, 8, 16 or 32; 0 when the property does not exist.
    pub format: u8,
    /// 0 (`None`) when the property does not exist.
    pub ty: Xid,
    pub bytes_after: u32,
    /// The value's bytes.
    pub value: Vec<u8>,
}

impl PropertyReply {
    /// Parse a whole reply (32 bytes and the value).
    pub fn parse(reply: &[u8]) -> PropertyReply {
        let format = u8_at(reply, 1);
        let unit = usize::from(format / 8);
        let n = u32_at(reply, 16) as usize * unit;
        let value = reply.get(32..32 + n).map(<[u8]>::to_vec).unwrap_or_default();
        PropertyReply { format, ty: u32_at(reply, 8), bytes_after: u32_at(reply, 12), value }
    }

    /// The value as 32-bit items (format 32), else empty.
    pub fn u32s(&self) -> Vec<u32> {
        if self.format != 32 {
            return Vec::new();
        }
        self.value.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }
}

/// The atom of an `InternAtom` reply.
pub fn intern_atom_reply(reply: &[u8]) -> Xid {
    u32_at(reply, 8)
}

/// The reply to `GetGeometry`: (x, y, width, height, border width).
pub fn geometry_reply(reply: &[u8]) -> (i16, i16, u16, u16, u16) {
    (u16_at(reply, 12) as i16, u16_at(reply, 14) as i16, u16_at(reply, 16), u16_at(reply, 18), u16_at(reply, 20))
}

/// A reply as a server sends it (for tests): `data` byte, sequence number,
/// and the body after the 8-byte header (padded to at least 24 bytes and
/// to a multiple of 4).
pub fn encode_reply(data: u8, sequence: u16, body: &[u8]) -> Vec<u8> {
    let mut b = body.to_vec();
    if b.len() < 24 {
        b.resize(24, 0);
    }
    b.resize(b.len() + pad(b.len()), 0);
    let mut out = Vec::new();
    Writer(&mut out).u8(1).u8(data).u16(sequence).u32(((b.len() - 24) / 4) as u32).bytes(&b);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn bytes(r: &Request<'_>) -> Vec<u8> {
        let mut v = Vec::new();
        r.encode(&mut v);
        v
    }

    #[test]
    fn create_window_sorts_its_value_list() {
        let r = Request::CreateWindow {
            depth: 0,
            wid: 0x0020_0001,
            parent: 0x3b6,
            x: -100,
            y: -100,
            width: 1,
            height: 1,
            border_width: 0,
            class: class::INPUT_ONLY,
            visual: 0,
            values: &[(cw::EVENT_MASK, event_mask::PROPERTY_CHANGE), (cw::OVERRIDE_REDIRECT, 1)],
        };
        let b = bytes(&r);
        assert_eq!(b.len(), 40);
        assert_eq!(&b[..4], [1, 0, 10, 0]);
        assert_eq!(u32_at(&b, 4), 0x0020_0001);
        assert_eq!(u32_at(&b, 8), 0x3b6);
        assert_eq!((u16_at(&b, 12) as i16, u16_at(&b, 14) as i16, u16_at(&b, 16), u16_at(&b, 18)), (-100, -100, 1, 1));
        assert_eq!((u16_at(&b, 20), u16_at(&b, 22), u32_at(&b, 24)), (0, 2, 0));
        assert_eq!(u32_at(&b, 28), 0x0a00, "override-redirect | event-mask");
        assert_eq!((u32_at(&b, 32), u32_at(&b, 36)), (1, event_mask::PROPERTY_CHANGE), "in bit order");
    }

    #[test]
    fn configure_window_has_a_sixteen_bit_mask() {
        let r = Request::ConfigureWindow {
            window: 7,
            values: &[(config::STACK_MODE, stack::ABOVE), (config::Y, 0xffff_ffe0), (config::X, 5)],
        };
        let b = bytes(&r);
        assert_eq!(b, [12, 0, 6, 0, 7, 0, 0, 0, 0x43, 0, 0, 0, 5, 0, 0, 0, 0xe0, 0xff, 0xff, 0xff, 0, 0, 0, 0]);
    }

    #[test]
    fn intern_atom_and_query_extension_pad_their_names() {
        let b = bytes(&Request::InternAtom { only_if_exists: false, name: b"WM_STATE" });
        assert_eq!(b.len(), 16);
        assert_eq!(&b[..8], [16, 0, 4, 0, 8, 0, 0, 0]);
        assert_eq!(&b[8..], b"WM_STATE");
        let b = bytes(&Request::InternAtom { only_if_exists: true, name: b"WL_SURFACE_ID" });
        assert_eq!((b.len(), b[1], u16_at(&b, 2), u16_at(&b, 4)), (24, 1, 6, 13));
        assert_eq!(&b[21..], [0, 0, 0]);
        let b = bytes(&Request::QueryExtension { name: b"XFIXES" });
        assert_eq!((b[0], u16_at(&b, 2), u16_at(&b, 4), b.len()), (98, 4, 6, 16));
    }

    #[test]
    fn change_property_counts_items_of_its_format() {
        let atoms = [5u32.to_le_bytes(), 9u32.to_le_bytes()].concat();
        let b = bytes(&Request::ChangeProperty {
            mode: prop_mode::REPLACE,
            window: 1,
            property: 2,
            ty: 4,
            format: 32,
            data: &atoms,
        });
        assert_eq!((b[0], b[1], u16_at(&b, 2)), (18, 0, 8));
        assert_eq!((u32_at(&b, 4), u32_at(&b, 8), u32_at(&b, 12), b[16]), (1, 2, 4, 32));
        assert_eq!(u32_at(&b, 20), 2, "two 32-bit items");
        assert_eq!((u32_at(&b, 24), u32_at(&b, 28)), (5, 9));
        let b = bytes(&Request::ChangeProperty {
            mode: prop_mode::APPEND,
            window: 1,
            property: 39,
            ty: 31,
            format: 8,
            data: b"xeyes",
        });
        assert_eq!((b[1], u16_at(&b, 2), u32_at(&b, 20), b.len()), (2, 8, 5, 32));
    }

    #[test]
    fn fixed_size_requests() {
        assert_eq!(bytes(&Request::MapWindow { window: 9 }), [8, 0, 2, 0, 9, 0, 0, 0]);
        assert_eq!(bytes(&Request::UnmapWindow { window: 9 })[0], 10);
        assert_eq!(bytes(&Request::GetInputFocus), [43, 0, 1, 0]);
        assert_eq!(bytes(&Request::ChangeSaveSet { insert: false, window: 3 })[..2], [6, 1]);
        assert_eq!(
            bytes(&Request::SetInputFocus { revert_to: revert::POINTER_ROOT, focus: 9, time: 0 }),
            [42, 1, 3, 0, 9, 0, 0, 0, 0, 0, 0, 0]
        );
        let b = bytes(&Request::GetProperty {
            delete: false,
            window: 9,
            property: 39,
            ty: 0,
            long_offset: 0,
            long_length: 64,
        });
        assert_eq!((b[0], u16_at(&b, 2), u32_at(&b, 20)), (20, 6, 64));
        let b = bytes(&Request::SendEvent { propagate: false, destination: 9, event_mask: 0, event: [33; 32] });
        assert_eq!((b[0], u16_at(&b, 2), b.len(), b[12]), (25, 11, 44, 33));
        let b = bytes(&Request::ConvertSelection { requestor: 1, selection: 2, target: 3, property: 4, time: 5 });
        assert_eq!((b[0], u16_at(&b, 2), u32_at(&b, 20)), (24, 6, 5));
        assert_eq!(bytes(&Request::KillClient { resource: 9 }), [113, 0, 2, 0, 9, 0, 0, 0]);
        assert!(Request::GetInputFocus.has_reply() && !Request::MapWindow { window: 1 }.has_reply());
    }

    #[test]
    fn extension_requests_carry_their_minor_opcode() {
        let body = composite::redirect_subwindows(0x3b6, composite::REDIRECT_MANUAL);
        let r = Request::Extension { major: 142, minor: composite::REDIRECT_SUBWINDOWS, body: &body, reply: false };
        assert_eq!(bytes(&r), [142, 2, 3, 0, 0xb6, 3, 0, 0, 1, 0, 0, 0]);
        assert!(!r.has_reply());
        let body = composite::query_version(0, 4);
        let r = Request::Extension { major: 142, minor: composite::QUERY_VERSION, body: &body, reply: true };
        assert_eq!(bytes(&r), [142, 0, 3, 0, 0, 0, 0, 0, 4, 0, 0, 0]);
        assert!(r.has_reply());
        // QueryExtension's reply: present at 8, major opcode at 9.
        let mut body = vec![0u8; 24];
        (body[0], body[1]) = (1, 142);
        assert_eq!(query_extension_reply(&encode_reply(0, 1, &body)), Some(142));
        assert_eq!(query_extension_reply(&encode_reply(0, 1, &[])), None);
    }

    #[test]
    fn parses_property_replies() {
        let mut body = vec![0u8; 24];
        body[0..4].copy_from_slice(&31u32.to_le_bytes()); // type STRING
        body[8..12].copy_from_slice(&5u32.to_le_bytes()); // 5 items
        body.extend_from_slice(b"xeyes");
        let reply = encode_reply(8, 3, &body);
        let p = PropertyReply::parse(&reply);
        assert_eq!((p.format, p.ty, p.value.as_slice()), (8, 31, &b"xeyes"[..]));
        assert!(p.u32s().is_empty());
        let mut body = vec![0u8; 24];
        body[0..4].copy_from_slice(&4u32.to_le_bytes());
        body[8..12].copy_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&[7, 0, 0, 0, 8, 0, 0, 0]);
        assert_eq!(PropertyReply::parse(&encode_reply(32, 4, &body)).u32s(), [7, 8]);
        // A property that does not exist: format 0, type None, no value.
        let p = PropertyReply::parse(&encode_reply(0, 5, &[]));
        assert_eq!((p.format, p.ty, p.value.len()), (0, 0, 0));
    }
}
