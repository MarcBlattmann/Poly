//! UDP (RFC 768): ports, length, checksum over a pseudo-header.
//!
//! Over IPv4 a zero checksum means "none was computed"; over IPv6 it is
//! invalid (RFC 8200 8.1). A computed checksum of zero is sent as all ones.

use super::checksum::Sum;
use super::{WireError, be16, put16};

pub const HEADER_LEN: usize = 8;

/// A parsed header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    /// Header plus data, from the length field.
    pub len: usize,
    pub checksum: u16,
}

/// Parse a datagram; the data is cut to the length field. `buf` is the IP
/// payload.
pub fn parse(buf: &[u8]) -> Result<(UdpHeader, &[u8]), WireError> {
    if buf.len() < HEADER_LEN {
        return Err(WireError::Truncated);
    }
    let len = usize::from(be16(buf, 4));
    if len < HEADER_LEN {
        return Err(WireError::Malformed);
    }
    if len > buf.len() {
        return Err(WireError::Truncated);
    }
    let h = UdpHeader { src_port: be16(buf, 0), dst_port: be16(buf, 2), len, checksum: be16(buf, 6) };
    Ok((h, &buf[HEADER_LEN..len]))
}

/// Verify the checksum of `buf[..h.len]` given the pseudo-header sum.
/// `optional` allows a zero (absent) checksum, as IPv4 does.
pub fn verify(buf: &[u8], h: &UdpHeader, pseudo: Sum, optional: bool) -> bool {
    if h.checksum == 0 {
        return optional;
    }
    pseudo.add_bytes(&buf[..h.len]).fold() == 0xffff
}

/// Write the header for data already at `buf[HEADER_LEN..HEADER_LEN + data_len]`.
pub fn emit(buf: &mut [u8], src_port: u16, dst_port: u16, data_len: usize, pseudo: Sum) {
    let len = HEADER_LEN + data_len;
    put16(buf, 0, src_port);
    put16(buf, 2, dst_port);
    put16(buf, 4, len as u16);
    put16(buf, 6, 0);
    let mut c = pseudo.add_bytes(&buf[..len]).finish();
    if c == 0 {
        c = 0xffff;
    }
    put16(buf, 6, c);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{checksum, proto};
    use core::net::Ipv4Addr;

    #[test]
    fn roundtrip_and_checksum() {
        let (s, d) = (Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2));
        let mut buf = [0u8; 13];
        buf[8..].copy_from_slice(b"hello");
        let ps = checksum::pseudo_v4(s, d, proto::UDP, 13);
        emit(&mut buf, 1000, 53, 5, ps);
        let (h, data) = parse(&buf).unwrap();
        assert_eq!((h.src_port, h.dst_port, h.len, data), (1000, 53, 13, &b"hello"[..]));
        assert!(verify(&buf, &h, ps, true));
        let mut bad = buf;
        bad[9] ^= 1;
        let (hb, _) = parse(&bad).unwrap();
        assert!(!verify(&bad, &hb, ps, true));
        let mut none = buf;
        none[6] = 0;
        none[7] = 0;
        let (hn, _) = parse(&none).unwrap();
        assert!(verify(&none, &hn, ps, true));
        assert!(!verify(&none, &hn, ps, false));
    }

    #[test]
    fn length_checks() {
        let mut buf = [0u8; 8];
        buf[5] = 7;
        assert_eq!(parse(&buf).unwrap_err(), WireError::Malformed);
        buf[5] = 9;
        assert_eq!(parse(&buf).unwrap_err(), WireError::Truncated);
        assert_eq!(parse(&buf[..7]).unwrap_err(), WireError::Truncated);
    }
}
