//! Ethernet II framing (RFC 894 for IPv4, RFC 2464 for IPv6): destination
//! and source MAC, EtherType, payload. The frame check sequence is the
//! device's business.

use super::{WireError, be16, put16};
use core::fmt;

/// Header length: two addresses and the EtherType.
pub const HEADER_LEN: usize = 14;
/// Shortest frame on the wire without the FCS (RFC 894: 46 bytes of data).
pub const MIN_FRAME_LEN: usize = 60;

/// EtherType of IPv4 (RFC 894).
pub const ETHERTYPE_IPV4: u16 = 0x0800;
/// EtherType of ARP (RFC 826).
pub const ETHERTYPE_ARP: u16 = 0x0806;
/// EtherType of IPv6 (RFC 2464).
pub const ETHERTYPE_IPV6: u16 = 0x86dd;

/// A 48-bit IEEE 802 MAC address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    /// ff:ff:ff:ff:ff:ff.
    pub const BROADCAST: MacAddr = MacAddr([0xff; 6]);
    /// 00:00:00:00:00:00.
    pub const ZERO: MacAddr = MacAddr([0; 6]);

    /// The group bit (least significant bit of the first octet).
    pub fn is_multicast(self) -> bool {
        self.0[0] & 1 != 0
    }

    /// All ones.
    pub fn is_broadcast(self) -> bool {
        self == MacAddr::BROADCAST
    }

    /// Neither group nor zero: a valid station address.
    pub fn is_unicast(self) -> bool {
        !self.is_multicast() && self != MacAddr::ZERO
    }

    /// The Ethernet multicast address for an IPv6 multicast address
    /// (RFC 2464 section 7: 33:33 followed by the last four octets).
    pub fn ipv6_multicast(addr: core::net::Ipv6Addr) -> MacAddr {
        let o = addr.octets();
        MacAddr([0x33, 0x33, o[12], o[13], o[14], o[15]])
    }

    /// The Ethernet multicast address for an IPv4 multicast address
    /// (RFC 1112 section 6.4: 01:00:5e and the low 23 bits).
    pub fn ipv4_multicast(addr: core::net::Ipv4Addr) -> MacAddr {
        let o = addr.octets();
        MacAddr([0x01, 0x00, 0x5e, o[1] & 0x7f, o[2], o[3]])
    }
}

impl fmt::Debug for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// A parsed frame.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ethertype: u16,
    /// Everything after the header, padding included.
    pub payload: &'a [u8],
}

/// Parse an Ethernet II frame. Lengths below 1536 in the type field are
/// IEEE 802.3 length fields, which this stack does not speak.
pub fn parse(buf: &[u8]) -> Result<Frame<'_>, WireError> {
    if buf.len() < HEADER_LEN {
        return Err(WireError::Truncated);
    }
    let ethertype = be16(buf, 12);
    if ethertype < 0x0600 {
        return Err(WireError::Unsupported);
    }
    let mut dst = [0; 6];
    let mut src = [0; 6];
    dst.copy_from_slice(&buf[0..6]);
    src.copy_from_slice(&buf[6..12]);
    Ok(Frame { dst: MacAddr(dst), src: MacAddr(src), ethertype, payload: &buf[HEADER_LEN..] })
}

/// Write a header into the first [`HEADER_LEN`] bytes of `buf`.
pub fn write_header(buf: &mut [u8], dst: MacAddr, src: MacAddr, ethertype: u16) {
    buf[0..6].copy_from_slice(&dst.0);
    buf[6..12].copy_from_slice(&src.0);
    put16(buf, 12, ethertype);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut buf = [0u8; 20];
        let a = MacAddr([2, 0, 0, 0, 0, 1]);
        write_header(&mut buf, MacAddr::BROADCAST, a, ETHERTYPE_ARP);
        let f = parse(&buf).unwrap();
        assert_eq!(f.dst, MacAddr::BROADCAST);
        assert_eq!(f.src, a);
        assert_eq!(f.ethertype, ETHERTYPE_ARP);
        assert_eq!(f.payload.len(), 6);
    }

    #[test]
    fn rejects_short_and_8023() {
        assert_eq!(parse(&[0; 13]).unwrap_err(), WireError::Truncated);
        let mut buf = [0u8; 14];
        buf[12] = 0x05;
        assert_eq!(parse(&buf).unwrap_err(), WireError::Unsupported);
    }

    #[test]
    fn address_classes() {
        assert!(MacAddr::BROADCAST.is_multicast());
        assert!(!MacAddr::ZERO.is_unicast());
        assert!(MacAddr([2, 0, 0, 0, 0, 1]).is_unicast());
        let m = MacAddr::ipv6_multicast("ff02::1:ff00:1234".parse().unwrap());
        assert_eq!(m, MacAddr([0x33, 0x33, 0xff, 0x00, 0x12, 0x34]));
        let m = MacAddr::ipv4_multicast("224.129.2.3".parse().unwrap());
        assert_eq!(m, MacAddr([1, 0, 0x5e, 1, 2, 3]));
    }
}
