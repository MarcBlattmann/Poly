//! The IPv6 header and extension-header walk (RFC 8200).
//!
//! Hop-by-Hop, Routing and Destination Options headers are skipped (their
//! options are not acted on); a Fragment header makes the packet
//! `Unsupported`, because this stack does not reassemble IPv6.

use super::{WireError, be16, be32, proto, put16, put32};
use core::net::Ipv6Addr;

/// Fixed header length.
pub const HEADER_LEN: usize = 40;
/// Every IPv6 link carries packets this big (RFC 8200 section 5).
pub const MIN_MTU: usize = 1280;
/// Bound on the extension-header chain walked per packet.
const MAX_EXTENSION_HEADERS: usize = 8;

/// A parsed (or to-be-emitted) fixed header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv6Header {
    pub traffic_class: u8,
    pub flow_label: u32,
    pub payload_len: usize,
    pub next_header: u8,
    pub hop_limit: u8,
    pub src: Ipv6Addr,
    pub dst: Ipv6Addr,
}

fn addr(b: &[u8]) -> Ipv6Addr {
    let mut o = [0u8; 16];
    o.copy_from_slice(&b[..16]);
    Ipv6Addr::from(o)
}

/// Parse the fixed header; the payload is cut to the payload length. Jumbograms
/// (payload length zero with a Jumbo option) are not supported.
pub fn parse(buf: &[u8]) -> Result<(Ipv6Header, &[u8]), WireError> {
    if buf.len() < HEADER_LEN {
        return Err(WireError::Truncated);
    }
    if buf[0] >> 4 != 6 {
        return Err(WireError::Malformed);
    }
    let vtf = be32(buf, 0);
    let payload_len = usize::from(be16(buf, 4));
    if HEADER_LEN + payload_len > buf.len() {
        return Err(WireError::Truncated);
    }
    let h = Ipv6Header {
        traffic_class: (vtf >> 20) as u8,
        flow_label: vtf & 0x000f_ffff,
        payload_len,
        next_header: buf[6],
        hop_limit: buf[7],
        src: addr(&buf[8..24]),
        dst: addr(&buf[24..40]),
    };
    Ok((h, &buf[HEADER_LEN..HEADER_LEN + payload_len]))
}

/// Parse the header quoted in an ICMPv6 error, which is usually cut short:
/// the payload is whatever follows, up to the payload length.
pub fn parse_quoted(buf: &[u8]) -> Result<(Ipv6Header, &[u8]), WireError> {
    if buf.len() < HEADER_LEN {
        return Err(WireError::Truncated);
    }
    let mut full = [0u8; HEADER_LEN];
    full.copy_from_slice(&buf[..HEADER_LEN]);
    full[4] = 0;
    full[5] = 0;
    let (mut h, _) = parse(&full)?;
    h.payload_len = usize::from(be16(buf, 4));
    let end = (HEADER_LEN + h.payload_len).min(buf.len());
    Ok((h, &buf[HEADER_LEN..end]))
}

/// Write the fixed header into `buf`.
pub fn emit(buf: &mut [u8], h: &Ipv6Header) {
    put32(buf, 0, (6 << 28) | (u32::from(h.traffic_class) << 20) | (h.flow_label & 0x000f_ffff));
    put16(buf, 4, h.payload_len as u16);
    buf[6] = h.next_header;
    buf[7] = h.hop_limit;
    buf[8..24].copy_from_slice(&h.src.octets());
    buf[24..40].copy_from_slice(&h.dst.octets());
}

/// Skip extension headers: returns the upper-layer protocol and the offset
/// of its header within `payload`.
pub fn skip_extensions(mut next: u8, payload: &[u8]) -> Result<(u8, usize), WireError> {
    let mut off = 0usize;
    for _ in 0..MAX_EXTENSION_HEADERS {
        match next {
            proto::HOPOPTS | proto::ROUTING | proto::DSTOPTS => {
                let h = payload.get(off..off + 2).ok_or(WireError::Truncated)?;
                let len = (usize::from(h[1]) + 1) * 8;
                if off + len > payload.len() {
                    return Err(WireError::Truncated);
                }
                next = h[0];
                off += len;
            }
            proto::FRAGMENT => return Err(WireError::Unsupported),
            _ => return Ok((next, off)),
        }
    }
    Err(WireError::Unsupported)
}

/// The solicited-node multicast address of `a` (RFC 4291 section 2.7.1).
pub fn solicited_node(a: Ipv6Addr) -> Ipv6Addr {
    let o = a.octets();
    Ipv6Addr::new(0xff02, 0, 0, 0, 0, 1, 0xff00 | u16::from(o[13]), u16::from_be_bytes([o[14], o[15]]))
}

/// The all-nodes link-local multicast address ff02::1.
pub const ALL_NODES: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);

/// The link-local address formed from a MAC by modified EUI-64
/// (RFC 4291 section 2.5.1 and appendix A, RFC 2464 section 5).
pub fn link_local_from_mac(mac: [u8; 6]) -> Ipv6Addr {
    let mut o = [0u8; 16];
    o[0] = 0xfe;
    o[1] = 0x80;
    o[8] = mac[0] ^ 0x02;
    o[9] = mac[1];
    o[10] = mac[2];
    o[11] = 0xff;
    o[12] = 0xfe;
    o[13] = mac[3];
    o[14] = mac[4];
    o[15] = mac[5];
    Ipv6Addr::from(o)
}

/// True for unicast link-local addresses (fe80::/10).
pub fn is_link_local(a: Ipv6Addr) -> bool {
    a.segments()[0] & 0xffc0 == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let h = Ipv6Header {
            traffic_class: 0xab,
            flow_label: 0x12345,
            payload_len: 4,
            next_header: proto::UDP,
            hop_limit: 64,
            src: "fe80::1".parse().unwrap(),
            dst: "fe80::2".parse().unwrap(),
        };
        let mut buf = [0u8; 48];
        emit(&mut buf, &h);
        let (p, payload) = parse(&buf).unwrap();
        assert_eq!(p, h);
        assert_eq!(payload.len(), 4);
        assert_eq!(parse(&buf[..43]).unwrap_err(), WireError::Truncated);
        buf[0] = 0x45;
        assert_eq!(parse(&buf).unwrap_err(), WireError::Malformed);
    }

    #[test]
    fn extension_walk() {
        // Hop-by-hop (8 bytes) then destination options (16 bytes) then UDP.
        let mut p = [0u8; 32];
        p[0] = proto::DSTOPTS;
        p[1] = 0;
        p[8] = proto::UDP;
        p[9] = 1;
        assert_eq!(skip_extensions(proto::HOPOPTS, &p), Ok((proto::UDP, 24)));
        assert_eq!(skip_extensions(proto::HOPOPTS, &p[..20]), Err(WireError::Truncated));
        assert_eq!(skip_extensions(proto::FRAGMENT, &p), Err(WireError::Unsupported));
        assert_eq!(skip_extensions(proto::TCP, &p), Ok((proto::TCP, 0)));
        // A loop of options headers that never ends is bounded.
        let p = [proto::HOPOPTS, 0, 0, 0, 0, 0, 0, 0].repeat(20);
        assert_eq!(skip_extensions(proto::HOPOPTS, &p), Err(WireError::Unsupported));
    }

    #[test]
    fn addresses() {
        let ll = link_local_from_mac([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        assert_eq!(ll, "fe80::5054:ff:fe12:3456".parse::<Ipv6Addr>().unwrap());
        assert!(is_link_local(ll));
        assert_eq!(solicited_node(ll), "ff02::1:ff12:3456".parse::<Ipv6Addr>().unwrap());
    }
}
