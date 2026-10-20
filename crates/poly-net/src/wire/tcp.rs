//! The TCP header (RFC 9293 section 3.1) and the options this stack speaks:
//! MSS (RFC 9293 3.2), window scale and timestamps (RFC 7323), SACK-permitted
//! (RFC 2018; recognised, never offered).

use super::checksum::Sum;
use super::{WireError, be16, be32, put16, put32};

pub const MIN_HEADER_LEN: usize = 20;
pub const MAX_HEADER_LEN: usize = 60;

pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;
pub const ACK: u8 = 0x10;
pub const URG: u8 = 0x20;

const OPT_END: u8 = 0;
const OPT_NOP: u8 = 1;
const OPT_MSS: u8 = 2;
const OPT_WSCALE: u8 = 3;
const OPT_SACK_PERMITTED: u8 = 4;
const OPT_TIMESTAMPS: u8 = 8;

/// The largest shift RFC 7323 2.3 allows; larger values are treated as 14.
pub const MAX_WSCALE: u8 = 14;

/// Options carried by a segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TcpOptions {
    pub mss: Option<u16>,
    pub wscale: Option<u8>,
    pub sack_permitted: bool,
    /// (TSval, TSecr).
    pub timestamps: Option<(u32, u32)>,
}

impl TcpOptions {
    /// Bytes these options take when emitted, padded to a multiple of four.
    pub fn len(&self) -> usize {
        let mut n = 0;
        if self.mss.is_some() {
            n += 4;
        }
        if self.wscale.is_some() {
            n += 4;
        }
        if self.sack_permitted {
            n += 4;
        }
        if self.timestamps.is_some() {
            n += 12;
        }
        n
    }

    /// True when no option is set.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn parse(mut b: &[u8]) -> Result<TcpOptions, WireError> {
        let mut o = TcpOptions::default();
        while let Some(&kind) = b.first() {
            match kind {
                OPT_END => break,
                OPT_NOP => {
                    b = &b[1..];
                    continue;
                }
                _ => {}
            }
            // RFC 9293 3.1: an option length below 2 or past the header is
            // illegal; the segment is dropped rather than guessed at.
            let len = usize::from(*b.get(1).ok_or(WireError::Malformed)?);
            if len < 2 || len > b.len() {
                return Err(WireError::Malformed);
            }
            let body = &b[2..len];
            match (kind, len) {
                (OPT_MSS, 4) => o.mss = Some(be16(body, 0)),
                (OPT_WSCALE, 3) => o.wscale = Some(body[0].min(MAX_WSCALE)),
                (OPT_SACK_PERMITTED, 2) => o.sack_permitted = true,
                (OPT_TIMESTAMPS, 10) => o.timestamps = Some((be32(body, 0), be32(body, 4))),
                (OPT_MSS | OPT_WSCALE | OPT_SACK_PERMITTED | OPT_TIMESTAMPS, _) => return Err(WireError::Malformed),
                _ => {} // Unknown options are skipped (RFC 9293 3.2).
            }
            b = &b[len..];
        }
        Ok(o)
    }

    fn emit(&self, b: &mut [u8]) {
        let mut i = 0;
        if let Some(mss) = self.mss {
            b[i] = OPT_MSS;
            b[i + 1] = 4;
            put16(b, i + 2, mss);
            i += 4;
        }
        if let Some(ws) = self.wscale {
            b[i] = OPT_NOP;
            b[i + 1] = OPT_WSCALE;
            b[i + 2] = 3;
            b[i + 3] = ws;
            i += 4;
        }
        if self.sack_permitted {
            b[i] = OPT_NOP;
            b[i + 1] = OPT_NOP;
            b[i + 2] = OPT_SACK_PERMITTED;
            b[i + 3] = 2;
            i += 4;
        }
        if let Some((val, ecr)) = self.timestamps {
            b[i] = OPT_NOP;
            b[i + 1] = OPT_NOP;
            b[i + 2] = OPT_TIMESTAMPS;
            b[i + 3] = 10;
            put32(b, i + 4, val);
            put32(b, i + 8, ecr);
        }
    }
}

/// A segment header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    pub urgent: u16,
    pub options: TcpOptions,
}

impl TcpHeader {
    /// True if every bit in `f` is set.
    pub fn has(&self, f: u8) -> bool {
        self.flags & f == f
    }

    /// Header length when emitted.
    pub fn header_len(&self) -> usize {
        MIN_HEADER_LEN + self.options.len()
    }
}

/// Parse a segment (the IP payload). The checksum is checked separately with
/// [`verify`], since it needs the pseudo-header.
pub fn parse(buf: &[u8]) -> Result<(TcpHeader, &[u8]), WireError> {
    if buf.len() < MIN_HEADER_LEN {
        return Err(WireError::Truncated);
    }
    let header_len = usize::from(buf[12] >> 4) * 4;
    if header_len < MIN_HEADER_LEN {
        return Err(WireError::Malformed);
    }
    if header_len > buf.len() {
        return Err(WireError::Truncated);
    }
    let options = TcpOptions::parse(&buf[MIN_HEADER_LEN..header_len])?;
    let h = TcpHeader {
        src_port: be16(buf, 0),
        dst_port: be16(buf, 2),
        seq: be32(buf, 4),
        ack: be32(buf, 8),
        flags: buf[13],
        window: be16(buf, 14),
        urgent: be16(buf, 18),
        options,
    };
    Ok((h, &buf[header_len..]))
}

/// True if the whole segment checksums correctly with `pseudo`.
pub fn verify(segment: &[u8], pseudo: Sum) -> bool {
    pseudo.add_bytes(segment).fold() == 0xffff
}

/// Write the header in front of data already at
/// `buf[h.header_len()..h.header_len() + data_len]` and checksum the segment.
/// Returns the segment length.
pub fn emit(buf: &mut [u8], h: &TcpHeader, data_len: usize, pseudo: Sum) -> usize {
    let hl = h.header_len();
    put16(buf, 0, h.src_port);
    put16(buf, 2, h.dst_port);
    put32(buf, 4, h.seq);
    put32(buf, 8, h.ack);
    buf[12] = ((hl / 4) as u8) << 4;
    buf[13] = h.flags;
    put16(buf, 14, h.window);
    put16(buf, 16, 0);
    put16(buf, 18, h.urgent);
    h.options.emit(&mut buf[MIN_HEADER_LEN..hl]);
    let len = hl + data_len;
    let c = pseudo.add_bytes(&buf[..len]).finish();
    put16(buf, 16, c);
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{checksum, proto};
    use core::net::Ipv4Addr;

    fn ps(len: usize) -> Sum {
        checksum::pseudo_v4(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2), proto::TCP, len)
    }

    #[test]
    fn roundtrip_with_all_options() {
        let h = TcpHeader {
            src_port: 1234,
            dst_port: 80,
            seq: 0xdead_beef,
            ack: 7,
            flags: SYN | ACK,
            window: 65535,
            urgent: 0,
            options: TcpOptions { mss: Some(1460), wscale: Some(7), sack_permitted: true, timestamps: Some((1, 2)) },
        };
        let mut buf = [0u8; 64];
        buf[h.header_len()..h.header_len() + 3].copy_from_slice(b"abc");
        let len = emit(&mut buf, &h, 3, ps(h.header_len() + 3));
        assert_eq!(len, 20 + 24 + 3);
        assert!(verify(&buf[..len], ps(len)));
        let (p, data) = parse(&buf[..len]).unwrap();
        assert_eq!(p, h);
        assert_eq!(data, b"abc");
        buf[30] ^= 1;
        assert!(!verify(&buf[..len], ps(len)));
    }

    #[test]
    fn malformed_options() {
        let mut buf = [0u8; 28];
        buf[12] = 7 << 4;
        // MSS with a bad length.
        buf[20..24].copy_from_slice(&[OPT_MSS, 3, 0, 0]);
        assert_eq!(parse(&buf).unwrap_err(), WireError::Malformed);
        // Zero length.
        buf[20..24].copy_from_slice(&[99, 0, 0, 0]);
        assert_eq!(parse(&buf).unwrap_err(), WireError::Malformed);
        // Length past the header.
        buf[20..24].copy_from_slice(&[99, 12, 0, 0]);
        assert_eq!(parse(&buf).unwrap_err(), WireError::Malformed);
        // Kind with no length byte.
        buf[20..28].copy_from_slice(&[1, 1, 1, 1, 1, 1, 1, 99]);
        assert_eq!(parse(&buf).unwrap_err(), WireError::Malformed);
        // Unknown option skipped, end of list stops parsing.
        buf[20..28].copy_from_slice(&[99, 4, 0, 0, 0, 0xff, 0xff, 0xff]);
        assert_eq!(parse(&buf).unwrap().0.options, TcpOptions::default());
        // Window scale above 14 is clamped.
        buf[20..28].copy_from_slice(&[OPT_WSCALE, 3, 20, 0, 0, 0, 0, 0]);
        assert_eq!(parse(&buf).unwrap().0.options.wscale, Some(14));
    }

    #[test]
    fn header_length_checks() {
        let mut buf = [0u8; 20];
        buf[12] = 4 << 4;
        assert_eq!(parse(&buf).unwrap_err(), WireError::Malformed);
        buf[12] = 6 << 4;
        assert_eq!(parse(&buf).unwrap_err(), WireError::Truncated);
        assert_eq!(parse(&buf[..19]).unwrap_err(), WireError::Truncated);
    }
}
