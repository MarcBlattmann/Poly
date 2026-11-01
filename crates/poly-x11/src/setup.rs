//! Connection setup ("X Window System Protocol", "Connection Setup" and
//! its encoding in appendix B).
//!
//! The client opens with byte order, protocol version 11.0 and an
//! authorization protocol name and data (none here: Xwayland's `-wm`
//! connection and local connections without `-auth` need none). The
//! server answers `Failed` (0), `Success` (1) or `Authenticate` (2), each
//! with an 8-byte header whose last field is the length of what follows
//! in 4-byte units.

use crate::wire::{Writer, pad, u8_at, u16_at, u32_at};
use alloc::string::String;
use alloc::vec::Vec;

/// The protocol version this client speaks.
pub const PROTOCOL_MAJOR: u16 = 11;
pub const PROTOCOL_MINOR: u16 = 0;

/// The setup request: little-endian, 11.0, no authorization.
pub fn request(out: &mut Vec<u8>) {
    Writer(out).u8(b'l').u8(0).u16(PROTOCOL_MAJOR).u16(PROTOCOL_MINOR).u16(0).u16(0).u16(0);
}

/// A screen of the setup reply (the parts a window manager uses).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Screen {
    pub root: u32,
    pub default_colormap: u32,
    pub white_pixel: u32,
    pub black_pixel: u32,
    /// The root window's event mask when the connection was made.
    pub current_input_masks: u32,
    pub width: u16,
    pub height: u16,
    pub width_mm: u16,
    pub height_mm: u16,
    pub root_visual: u32,
    pub root_depth: u8,
}

/// The successful reply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub release: u32,
    pub resource_id_base: u32,
    pub resource_id_mask: u32,
    pub vendor: String,
    /// In 4-byte units.
    pub maximum_request_length: u16,
    pub min_keycode: u8,
    pub max_keycode: u8,
    pub screens: Vec<Screen>,
}

/// What the server answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Success(Setup),
    /// `Failed`, with the server's reason.
    Failed(String),
    /// `Authenticate`, with the server's reason.
    Authenticate(String),
}

/// A malformed setup reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed(pub &'static str);

/// Parse the setup reply at the start of `b`: `Ok(None)` until all of it
/// is there, else the reply and how many bytes it took.
pub fn parse(b: &[u8]) -> Result<Option<(Reply, usize)>, Malformed> {
    if b.len() < 8 {
        return Ok(None);
    }
    let total = 8 + 4 * usize::from(u16_at(b, 6));
    if b.len() < total {
        return Ok(None);
    }
    let body = &b[..total];
    let text = |at: usize, n: usize| String::from_utf8_lossy(body.get(at..at + n).unwrap_or_default()).into_owned();
    let reply = match b[0] {
        0 => Reply::Failed(text(8, usize::from(b[1]))),
        2 => {
            let reason = text(8, total - 8);
            Reply::Authenticate(String::from(reason.trim_end_matches('\0')))
        }
        1 => Reply::Success(success(body)?),
        _ => return Err(Malformed("unknown setup reply status")),
    };
    Ok(Some((reply, total)))
}

fn success(b: &[u8]) -> Result<Setup, Malformed> {
    if b.len() < 40 {
        return Err(Malformed("setup reply too short"));
    }
    let vendor_len = usize::from(u16_at(b, 24));
    let screens = usize::from(u8_at(b, 28));
    let formats = usize::from(u8_at(b, 29));
    let vendor_at = 40;
    let vendor = b.get(vendor_at..vendor_at + vendor_len).ok_or(Malformed("vendor past the reply"))?;
    let mut at = vendor_at + vendor_len + pad(vendor_len) + 8 * formats;
    let mut setup = Setup {
        protocol_major: u16_at(b, 2),
        protocol_minor: u16_at(b, 4),
        release: u32_at(b, 8),
        resource_id_base: u32_at(b, 12),
        resource_id_mask: u32_at(b, 16),
        vendor: String::from_utf8_lossy(vendor).into_owned(),
        maximum_request_length: u16_at(b, 26),
        min_keycode: u8_at(b, 34),
        max_keycode: u8_at(b, 35),
        screens: Vec::new(),
    };
    for _ in 0..screens {
        let s = b.get(at..at + 40).ok_or(Malformed("screen past the reply"))?;
        setup.screens.push(Screen {
            root: u32_at(s, 0),
            default_colormap: u32_at(s, 4),
            white_pixel: u32_at(s, 8),
            black_pixel: u32_at(s, 12),
            current_input_masks: u32_at(s, 16),
            width: u16_at(s, 20),
            height: u16_at(s, 22),
            width_mm: u16_at(s, 24),
            height_mm: u16_at(s, 26),
            root_visual: u32_at(s, 32),
            root_depth: u8_at(s, 38),
        });
        let depths = usize::from(u8_at(s, 39));
        at += 40;
        for _ in 0..depths {
            let d = b.get(at..at + 8).ok_or(Malformed("depth past the reply"))?;
            at += 8 + 24 * usize::from(u16_at(d, 2));
        }
        if at > b.len() {
            return Err(Malformed("visuals past the reply"));
        }
    }
    Ok(setup)
}

/// A setup reply as a server sends it (for tests of clients): one screen
/// with one depth and one TrueColor visual.
pub fn encode_success(setup: &Setup) -> Vec<u8> {
    let mut body = Vec::new();
    let mut w = Writer(&mut body);
    let vendor = setup.vendor.as_bytes();
    w.u32(setup.release)
        .u32(setup.resource_id_base)
        .u32(setup.resource_id_mask)
        .u32(256)
        .u16(vendor.len() as u16)
        .u16(setup.maximum_request_length)
        .u8(setup.screens.len() as u8)
        .u8(1)
        .u8(0)
        .u8(0)
        .u8(32)
        .u8(32)
        .u8(setup.min_keycode)
        .u8(setup.max_keycode)
        .zeros(4)
        .padded(vendor);
    // One pixmap format: depth 24, 32 bits per pixel, scanlines padded to 32.
    w.u8(24).u8(32).u8(32).zeros(5);
    for s in &setup.screens {
        w.u32(s.root)
            .u32(s.default_colormap)
            .u32(s.white_pixel)
            .u32(s.black_pixel)
            .u32(s.current_input_masks)
            .u16(s.width)
            .u16(s.height)
            .u16(s.width_mm)
            .u16(s.height_mm)
            .u16(1)
            .u16(1)
            .u32(s.root_visual)
            .u8(0)
            .u8(0)
            .u8(s.root_depth)
            .u8(1);
        // The depth with one visual: TrueColor (4), 8 bits per RGB value.
        w.u8(s.root_depth).u8(0).u16(1).zeros(4);
        w.u32(s.root_visual).u8(4).u8(8).u16(256).u32(0xff_0000).u32(0xff00).u32(0xff).zeros(4);
    }
    let mut out = Vec::new();
    Writer(&mut out).u8(1).u8(0).u16(setup.protocol_major).u16(setup.protocol_minor).u16((body.len() / 4) as u16);
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Setup {
        Setup {
            protocol_major: 11,
            protocol_minor: 0,
            release: 12_302_006,
            resource_id_base: 0x0020_0000,
            resource_id_mask: 0x001f_ffff,
            vendor: "The X.Org Foundation".into(),
            maximum_request_length: 65535,
            min_keycode: 8,
            max_keycode: 255,
            screens: alloc::vec![Screen {
                root: 0x3b6,
                default_colormap: 0x21,
                white_pixel: 0xff_ffff,
                black_pixel: 0,
                current_input_masks: 0,
                width: 1280,
                height: 800,
                width_mm: 338,
                height_mm: 211,
                root_visual: 0x21,
                root_depth: 24,
            }],
        }
    }

    #[test]
    fn the_request_is_little_endian_eleven_zero_without_authorization() {
        let mut v = Vec::new();
        request(&mut v);
        assert_eq!(v, [b'l', 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn parses_a_success_reply_once_it_is_all_there() {
        let bytes = encode_success(&sample());
        assert_eq!(bytes.len() % 4, 0);
        for n in [0, 7, 8, bytes.len() - 1] {
            assert_eq!(parse(&bytes[..n]), Ok(None), "{n} bytes");
        }
        let mut more = bytes.clone();
        more.extend_from_slice(&[1, 2, 3]);
        let (reply, used) = parse(&more).unwrap().unwrap();
        assert_eq!(used, bytes.len());
        assert_eq!(reply, Reply::Success(sample()));
    }

    #[test]
    fn parses_failed_and_authenticate() {
        // Failed: reason length 5, version 11.0, 2 words of reason.
        let b = [0, 5, 11, 0, 0, 0, 2, 0, b'n', b'o', b' ', b'w', b'm', 0, 0, 0];
        assert_eq!(parse(&b), Ok(Some((Reply::Failed("no wm".into()), 16))));
        let b = [2, 0, 0, 0, 0, 0, 1, 0, b'a', b'u', b't', 0];
        assert_eq!(parse(&b), Ok(Some((Reply::Authenticate("aut".into()), 12))));
        assert!(parse(&[7, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn rejects_screens_past_the_reply() {
        let mut bytes = encode_success(&sample());
        bytes[28] = 3; // three screens, one there
        assert!(parse(&bytes).is_err());
    }
}
