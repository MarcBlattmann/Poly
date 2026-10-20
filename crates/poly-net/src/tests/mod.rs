//! Whole-stack tests over the simulated link.

mod arp;
mod frag;
mod fuzz;
mod icmp;
mod ipv6;
mod syscalls;
mod tcp;
mod tcp_loss;
mod udp;

extern crate std;

use crate::sim::{Dir, MAC_A, MAC_B, Sim};
use crate::wire::checksum::{self, Sum};
use crate::wire::{ethernet, icmpv4, ipv4, proto};
use crate::wire::{ipv6 as wire_ipv6, udp as wire_udp};
use alloc::vec;
use alloc::vec::Vec;
use core::net::Ipv4Addr;

/// Deterministic test data: position-dependent so misordering shows.
pub fn pattern(n: usize, seed: u64) -> Vec<u8> {
    let mut r = crate::sim::Rng::new(seed);
    (0..n).map(|i| (r.next() as u8) ^ (i as u8)).collect()
}

/// Wrap an IP packet in a frame from B to A (or A to B).
pub fn frame(dir: Dir, ethertype: u16, packet: &[u8]) -> Vec<u8> {
    let (dst, src) = match dir {
        Dir::BtoA => (MAC_A, MAC_B),
        Dir::AtoB => (MAC_B, MAC_A),
    };
    let mut f = vec![0u8; 14 + packet.len()];
    ethernet::write_header(&mut f, dst, src, ethertype);
    f[14..].copy_from_slice(packet);
    if f.len() < 60 {
        f.resize(60, 0);
    }
    f
}

/// Put a frame straight into a stack's receive queue.
pub fn inject(s: &mut Sim, dir: Dir, f: Vec<u8>) {
    match dir {
        Dir::BtoA => s.da.inbox.push_back(f),
        Dir::AtoB => s.db.inbox.push_back(f),
    }
}

/// An IPv4 packet with the given header fields around `payload`.
pub fn ipv4_packet(h: &ipv4::Ipv4Header, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; 20 + payload.len()];
    let mut h = *h;
    h.header_len = 20;
    h.total_len = 20 + payload.len();
    ipv4::emit(&mut p, &h);
    p[20..].copy_from_slice(payload);
    p
}

/// A UDP datagram (header and data) between two IPv4 addresses.
pub fn udp_v4(src: Ipv4Addr, sport: u16, dst: Ipv4Addr, dport: u16, data: &[u8]) -> Vec<u8> {
    let mut u = vec![0u8; 8 + data.len()];
    u[8..].copy_from_slice(data);
    wire_udp::emit(&mut u, sport, dport, data.len(), checksum::pseudo_v4(src, dst, proto::UDP, 8 + data.len()));
    u
}

/// The ICMPv4 messages in a frame log going `dir`: (type, code, data).
pub fn icmp_in_log(log: &[(Dir, Vec<u8>)], dir: Dir) -> Vec<(u8, u8, Vec<u8>)> {
    log.iter()
        .filter(|(d, _)| *d == dir)
        .filter_map(|(_, f)| {
            let e = ethernet::parse(f).ok()?;
            let (h, p) = ipv4::parse(e.payload).ok()?;
            (h.protocol == proto::ICMP).then_some(())?;
            let m = icmpv4::parse(p).ok()?;
            Some((m.ty, m.code, m.data.to_vec()))
        })
        .collect()
}

/// Recompute every checksum in a frame (after a test mangled it), so the
/// damage reaches the deeper parsers instead of being caught early.
pub fn fix_checksums(f: &mut [u8]) {
    let Ok(e) = ethernet::parse(f) else { return };
    let et = e.ethertype;
    let ip = &mut f[14..];
    match et {
        ethernet::ETHERTYPE_IPV4 => {
            if ip.len() < 20 || ip[0] >> 4 != 4 {
                return;
            }
            let hl = usize::from(ip[0] & 0xf) * 4;
            let total = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
            if hl < 20 || hl > ip.len() {
                return;
            }
            ip[10] = 0;
            ip[11] = 0;
            let c = checksum::checksum(&ip[..hl]);
            ip[10..12].copy_from_slice(&c.to_be_bytes());
            let end = total.clamp(hl, ip.len());
            let src = Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]);
            let dst = Ipv4Addr::new(ip[16], ip[17], ip[18], ip[19]);
            let protocol = ip[9];
            let seg = &mut ip[hl..end];
            let ps = checksum::pseudo_v4(src, dst, protocol, seg.len());
            fix_transport(seg, protocol, ps);
        }
        ethernet::ETHERTYPE_IPV6 => {
            let Ok((h, _)) = wire_ipv6::parse(ip) else { return };
            let end = (40 + h.payload_len).min(ip.len());
            let seg = &mut ip[40..end];
            let ps = checksum::pseudo_v6(h.src, h.dst, h.next_header, seg.len());
            fix_transport(seg, h.next_header, ps);
        }
        _ => {}
    }
}

fn fix_transport(seg: &mut [u8], protocol: u8, ps: Sum) {
    let at = match protocol {
        proto::TCP if seg.len() >= 20 => 16,
        proto::UDP if seg.len() >= 8 => 6,
        proto::ICMP | proto::ICMPV6 if seg.len() >= 8 => 2,
        _ => return,
    };
    seg[at] = 0;
    seg[at + 1] = 0;
    let c = if protocol == proto::ICMP { checksum::checksum(seg) } else { ps.add_bytes(seg).finish() };
    seg[at..at + 2].copy_from_slice(&c.to_be_bytes());
}
