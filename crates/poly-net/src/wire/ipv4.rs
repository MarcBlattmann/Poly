//! The IPv4 header (RFC 791 section 3.1, RFC 1122 section 3.2.1).
//!
//! Received options are length-checked and otherwise ignored; this stack
//! never sends options.

use super::checksum;
use super::{WireError, be16, put16};
use core::net::Ipv4Addr;

/// Header length without options.
pub const MIN_HEADER_LEN: usize = 20;
/// Largest datagram the 16-bit total length can describe.
pub const MAX_PACKET_LEN: usize = 65535;
/// Every IPv4 host must accept datagrams this big (RFC 791).
pub const MIN_MTU: usize = 68;

/// A parsed (or to-be-emitted) header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Header {
    /// Header length in bytes, options included (always 20 when emitting).
    pub header_len: usize,
    /// DSCP and ECN.
    pub tos: u8,
    /// Header plus payload, from the total length field.
    pub total_len: usize,
    pub ident: u16,
    pub dont_fragment: bool,
    pub more_fragments: bool,
    /// Fragment offset in bytes (the field counts 8-byte units).
    pub frag_offset: usize,
    pub ttl: u8,
    pub protocol: u8,
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
}

impl Ipv4Header {
    /// True for any piece of a fragmented datagram.
    pub fn is_fragment(&self) -> bool {
        self.more_fragments || self.frag_offset != 0
    }

    /// A header for a new, unfragmented datagram carrying `payload_len` bytes.
    pub fn new(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, ttl: u8, payload_len: usize) -> Ipv4Header {
        Ipv4Header {
            header_len: MIN_HEADER_LEN,
            tos: 0,
            total_len: MIN_HEADER_LEN + payload_len,
            ident: 0,
            dont_fragment: false,
            more_fragments: false,
            frag_offset: 0,
            ttl,
            protocol,
            src,
            dst,
        }
    }
}

fn addr(b: &[u8]) -> Ipv4Addr {
    Ipv4Addr::new(b[0], b[1], b[2], b[3])
}

/// Parse and validate a header: version, lengths and checksum. The payload
/// is cut to the total length, dropping link-layer padding.
pub fn parse(buf: &[u8]) -> Result<(Ipv4Header, &[u8]), WireError> {
    if buf.len() < MIN_HEADER_LEN {
        return Err(WireError::Truncated);
    }
    if buf[0] >> 4 != 4 {
        return Err(WireError::Malformed);
    }
    let header_len = usize::from(buf[0] & 0xf) * 4;
    if header_len < MIN_HEADER_LEN {
        return Err(WireError::Malformed);
    }
    if buf.len() < header_len {
        return Err(WireError::Truncated);
    }
    let total_len = usize::from(be16(buf, 2));
    if total_len < header_len {
        return Err(WireError::Malformed);
    }
    if total_len > buf.len() {
        return Err(WireError::Truncated);
    }
    if !checksum::verify(&buf[..header_len]) {
        return Err(WireError::Checksum);
    }
    let flags_frag = be16(buf, 6);
    let header = Ipv4Header {
        header_len,
        tos: buf[1],
        total_len,
        ident: be16(buf, 4),
        dont_fragment: flags_frag & 0x4000 != 0,
        more_fragments: flags_frag & 0x2000 != 0,
        frag_offset: usize::from(flags_frag & 0x1fff) * 8,
        ttl: buf[8],
        protocol: buf[9],
        src: addr(&buf[12..16]),
        dst: addr(&buf[16..20]),
    };
    Ok((header, &buf[header_len..total_len]))
}

/// Write a 20-byte header (no options) with its checksum into `buf`.
pub fn emit(buf: &mut [u8], h: &Ipv4Header) {
    buf[0] = 0x45;
    buf[1] = h.tos;
    put16(buf, 2, h.total_len as u16);
    put16(buf, 4, h.ident);
    let mut flags_frag = ((h.frag_offset / 8) as u16) & 0x1fff;
    if h.dont_fragment {
        flags_frag |= 0x4000;
    }
    if h.more_fragments {
        flags_frag |= 0x2000;
    }
    put16(buf, 6, flags_frag);
    buf[8] = h.ttl;
    buf[9] = h.protocol;
    put16(buf, 10, 0);
    buf[12..16].copy_from_slice(&h.src.octets());
    buf[16..20].copy_from_slice(&h.dst.octets());
    let c = checksum::checksum(&buf[..MIN_HEADER_LEN]);
    put16(buf, 10, c);
}

/// True for addresses a datagram may come from (RFC 1122 3.2.1.3: not
/// broadcast, not multicast, not class E).
pub fn is_valid_source(a: Ipv4Addr) -> bool {
    !a.is_broadcast() && !a.is_multicast() && a.octets()[0] < 240
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Ipv4Header {
        let mut h = Ipv4Header::new(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2), 17, 64, 8);
        h.ident = 0x1234;
        h.more_fragments = true;
        h.frag_offset = 1480;
        h
    }

    #[test]
    fn roundtrip() {
        let mut buf = [0u8; 40];
        let h = sample();
        emit(&mut buf, &h);
        let (p, payload) = parse(&buf).unwrap();
        assert_eq!(p, h);
        assert_eq!(payload.len(), 8);
        assert!(p.is_fragment());
    }

    #[test]
    fn rejects_bad_headers() {
        let mut buf = [0u8; 40];
        emit(&mut buf, &sample());
        assert_eq!(parse(&buf[..19]).unwrap_err(), WireError::Truncated);
        assert_eq!(parse(&buf[..27]).unwrap_err(), WireError::Truncated);
        let mut b = buf;
        b[0] = 0x65;
        assert_eq!(parse(&b).unwrap_err(), WireError::Malformed);
        let mut b = buf;
        b[0] = 0x44;
        assert_eq!(parse(&b).unwrap_err(), WireError::Malformed);
        let mut b = buf;
        b[0] = 0x4f;
        assert_eq!(parse(&b).unwrap_err(), WireError::Truncated);
        let mut b = buf;
        b[3] = 10;
        assert_eq!(parse(&b).unwrap_err(), WireError::Malformed);
        let mut b = buf;
        b[8] ^= 1;
        assert_eq!(parse(&b).unwrap_err(), WireError::Checksum);
    }

    #[test]
    fn source_validity() {
        assert!(is_valid_source(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(!is_valid_source(Ipv4Addr::BROADCAST));
        assert!(!is_valid_source(Ipv4Addr::new(224, 0, 0, 1)));
        assert!(!is_valid_source(Ipv4Addr::new(240, 0, 0, 1)));
    }
}
