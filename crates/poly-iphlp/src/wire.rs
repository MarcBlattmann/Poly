//! The interface list on the wire: `\Device\Afd`'s `IF_LIST` request
//! (`poly_winsock::afd::IF_LIST`, Poly's own protocol between `iphlpapi`
//! and the NT personality). A header of 8 bytes (the count, then padding)
//! and `count` records of [`RECORD_SIZE`] bytes, little-endian:
//!
//! | Offset | Size | Field |
//! |---:|---:|---|
//! | 0 | 4 | `IfIndex` |
//! | 4 | 4 | kind: 0 loopback, 1 Ethernet |
//! | 8 | 4 | flags: bit 0 up, bit 1 has IPv4, bit 2 has gateway, bit 3 has IPv6 |
//! | 12 | 4 | MTU |
//! | 16 | 8 | link speed, bits per second |
//! | 24 | 8 | MAC (6 bytes, 2 padding) |
//! | 32 | 4 | IPv4 address |
//! | 36 | 1 | IPv4 prefix length |
//! | 37 | 1 | DNS server count (0..=3) |
//! | 38 | 1 | IPv6 prefix length |
//! | 39 | 1 | padding |
//! | 40 | 4 | gateway |
//! | 44 | 12 | three DNS servers |
//! | 56 | 16 | IPv6 address |

use crate::{IfInfo, Kind};
use alloc::vec::Vec;

pub const HEADER_SIZE: usize = 8;
pub const RECORD_SIZE: usize = 72;
/// More interfaces than this are not sent.
pub const MAX_INTERFACES: usize = 8;

const UP: u32 = 1;
const HAS_V4: u32 = 2;
const HAS_GW: u32 = 4;
const HAS_V6: u32 = 8;

/// The bytes for a list of interfaces.
pub fn encode(list: &[IfInfo]) -> Vec<u8> {
    let list = &list[..list.len().min(MAX_INTERFACES)];
    let mut b = alloc::vec![0u8; HEADER_SIZE + list.len() * RECORD_SIZE];
    b[0..4].copy_from_slice(&(list.len() as u32).to_le_bytes());
    for (i, f) in list.iter().enumerate() {
        let r = &mut b[HEADER_SIZE + i * RECORD_SIZE..HEADER_SIZE + (i + 1) * RECORD_SIZE];
        r[0..4].copy_from_slice(&f.index.to_le_bytes());
        r[4..8].copy_from_slice(&(if f.kind == Kind::Loopback { 0u32 } else { 1 }).to_le_bytes());
        let mut flags = 0;
        if f.up {
            flags |= UP;
        }
        if f.ipv4.is_some() {
            flags |= HAS_V4;
        }
        if f.gateway.is_some() {
            flags |= HAS_GW;
        }
        if f.ipv6.is_some() {
            flags |= HAS_V6;
        }
        r[8..12].copy_from_slice(&flags.to_le_bytes());
        r[12..16].copy_from_slice(&f.mtu.to_le_bytes());
        r[16..24].copy_from_slice(&f.speed.to_le_bytes());
        r[24..30].copy_from_slice(&f.mac);
        if let Some((a, p)) = f.ipv4 {
            r[32..36].copy_from_slice(&a);
            r[36] = p;
        }
        r[37] = f.dns.len().min(3) as u8;
        if let Some((a, p)) = f.ipv6 {
            r[56..72].copy_from_slice(&a);
            r[38] = p;
        }
        if let Some(g) = f.gateway {
            r[40..44].copy_from_slice(&g);
        }
        for (k, d) in f.dns.iter().take(3).enumerate() {
            r[44 + k * 4..48 + k * 4].copy_from_slice(d);
        }
    }
    b
}

/// The interfaces in `b` (`None` if it is cut short or malformed).
pub fn decode(b: &[u8]) -> Option<Vec<IfInfo>> {
    let n = u32::from_le_bytes(b.get(0..4)?.try_into().ok()?) as usize;
    if n > MAX_INTERFACES || b.len() < HEADER_SIZE + n * RECORD_SIZE {
        return None;
    }
    let mut v = Vec::new();
    for i in 0..n {
        let r = &b[HEADER_SIZE + i * RECORD_SIZE..HEADER_SIZE + (i + 1) * RECORD_SIZE];
        let u32_at = |at: usize| u32::from_le_bytes(r[at..at + 4].try_into().unwrap());
        let kind = match u32_at(4) {
            0 => Kind::Loopback,
            1 => Kind::Ethernet,
            _ => return None,
        };
        let flags = u32_at(8);
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&r[24..30]);
        let dns_count = usize::from(r[37]).min(3);
        let mut dns = Vec::new();
        for k in 0..dns_count {
            dns.push(r[44 + k * 4..48 + k * 4].try_into().unwrap());
        }
        v.push(IfInfo {
            index: u32_at(0),
            kind,
            mac,
            mtu: u32_at(12),
            up: flags & UP != 0,
            speed: u64::from_le_bytes(r[16..24].try_into().unwrap()),
            ipv4: (flags & HAS_V4 != 0).then(|| (r[32..36].try_into().unwrap(), r[36])),
            gateway: (flags & HAS_GW != 0).then(|| r[40..44].try_into().unwrap()),
            dns,
            ipv6: (flags & HAS_V6 != 0).then(|| (r[56..72].try_into().unwrap(), r[38])),
        });
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::eth;

    #[test]
    fn a_list_round_trips() {
        let list = alloc::vec![IfInfo::loopback(), eth()];
        let b = encode(&list);
        assert_eq!(b.len(), HEADER_SIZE + 2 * RECORD_SIZE);
        assert_eq!(decode(&b), Some(list));
    }

    #[test]
    fn short_or_bad_input_is_refused() {
        let b = encode(&[eth()]);
        assert_eq!(decode(&b[..b.len() - 1]), None);
        assert_eq!(decode(&[]), None);
        let mut bad = b.clone();
        bad[HEADER_SIZE + 4] = 7; // kind
        assert_eq!(decode(&bad), None);
        let mut many = b;
        many[0] = 200;
        assert_eq!(decode(&many), None);
    }

    #[test]
    fn at_most_eight_interfaces_are_sent() {
        let list: Vec<IfInfo> = (0..12).map(|_| eth()).collect();
        assert_eq!(decode(&encode(&list)).unwrap().len(), MAX_INTERFACES);
    }
}
