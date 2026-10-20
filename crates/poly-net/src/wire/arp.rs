//! ARP for IPv4 over Ethernet (RFC 826).
//!
//! Layout: hardware type (1 = Ethernet), protocol type (0x0800), hardware
//! length (6), protocol length (4), opcode, sender MAC, sender IP, target
//! MAC, target IP: 28 bytes.

use super::ethernet::{ETHERTYPE_IPV4, MacAddr};
use super::{WireError, be16, put16};
use core::net::Ipv4Addr;

/// Length of an Ethernet/IPv4 ARP packet.
pub const LEN: usize = 28;
/// `ares_op$REQUEST`.
pub const OP_REQUEST: u16 = 1;
/// `ares_op$REPLY`.
pub const OP_REPLY: u16 = 2;
const HTYPE_ETHERNET: u16 = 1;

/// A parsed ARP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArpPacket {
    pub op: u16,
    pub sender_mac: MacAddr,
    pub sender_ip: Ipv4Addr,
    pub target_mac: MacAddr,
    pub target_ip: Ipv4Addr,
}

fn ip(b: &[u8]) -> Ipv4Addr {
    Ipv4Addr::new(b[0], b[1], b[2], b[3])
}

fn mac(b: &[u8]) -> MacAddr {
    let mut m = [0; 6];
    m.copy_from_slice(&b[..6]);
    MacAddr(m)
}

impl ArpPacket {
    /// Parse; anything but Ethernet/IPv4 request or reply is `Unsupported`.
    pub fn parse(buf: &[u8]) -> Result<ArpPacket, WireError> {
        if buf.len() < LEN {
            return Err(WireError::Truncated);
        }
        if be16(buf, 0) != HTYPE_ETHERNET || be16(buf, 2) != ETHERTYPE_IPV4 || buf[4] != 6 || buf[5] != 4 {
            return Err(WireError::Unsupported);
        }
        let op = be16(buf, 6);
        if op != OP_REQUEST && op != OP_REPLY {
            return Err(WireError::Unsupported);
        }
        Ok(ArpPacket {
            op,
            sender_mac: mac(&buf[8..14]),
            sender_ip: ip(&buf[14..18]),
            target_mac: mac(&buf[18..24]),
            target_ip: ip(&buf[24..28]),
        })
    }

    /// Write the 28-byte packet into `buf`.
    pub fn emit(&self, buf: &mut [u8]) {
        put16(buf, 0, HTYPE_ETHERNET);
        put16(buf, 2, ETHERTYPE_IPV4);
        buf[4] = 6;
        buf[5] = 4;
        put16(buf, 6, self.op);
        buf[8..14].copy_from_slice(&self.sender_mac.0);
        buf[14..18].copy_from_slice(&self.sender_ip.octets());
        buf[18..24].copy_from_slice(&self.target_mac.0);
        buf[24..28].copy_from_slice(&self.target_ip.octets());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_rejects() {
        let p = ArpPacket {
            op: OP_REQUEST,
            sender_mac: MacAddr([2, 0, 0, 0, 0, 1]),
            sender_ip: Ipv4Addr::new(10, 0, 0, 1),
            target_mac: MacAddr::ZERO,
            target_ip: Ipv4Addr::new(10, 0, 0, 2),
        };
        let mut buf = [0u8; LEN];
        p.emit(&mut buf);
        assert_eq!(ArpPacket::parse(&buf).unwrap(), p);
        assert_eq!(ArpPacket::parse(&buf[..27]).unwrap_err(), WireError::Truncated);
        let mut bad = buf;
        bad[7] = 3;
        assert_eq!(ArpPacket::parse(&bad).unwrap_err(), WireError::Unsupported);
        let mut bad = buf;
        bad[5] = 16;
        assert_eq!(ArpPacket::parse(&bad).unwrap_err(), WireError::Unsupported);
    }
}
