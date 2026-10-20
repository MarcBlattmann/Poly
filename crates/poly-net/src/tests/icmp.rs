//! ICMP: ping sockets, echo replies, error generation and its limits, and
//! errors reaching TCP (path MTU).

use super::{frame, icmp_in_log, inject, ipv4_packet, pattern, udp_v4};
use crate::sim::{Dir, IP_A, IP_B, Sim};
use crate::wire::{ethernet, icmpv4, ipv4, proto};
use crate::{Domain, Errno, Protocol, Readiness, RecvFlags, SockType};
use alloc::vec;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;

fn echo_request(seq: u16, data: &[u8]) -> Vec<u8> {
    let mut m = vec![0u8; 8 + data.len()];
    m[0] = icmpv4::ECHO_REQUEST;
    m[6..8].copy_from_slice(&seq.to_be_bytes());
    m[8..].copy_from_slice(data);
    m
}

#[test]
fn ping_socket_round_trip() {
    let mut s = Sim::new(50);
    let p = s.a.socket(Domain::Inet, SockType::Datagram, Some(Protocol::Icmp)).unwrap();
    for seq in 1..=3u16 {
        let req = echo_request(seq, b"0123456789abcdef");
        s.a.send_to(p, &req, Some(SocketAddr::new(IP_B.into(), 0))).unwrap();
        assert!(s.run_until(Duration::from_secs(1), |s| s.a.readiness(p).unwrap().contains(Readiness::READABLE)));
        let mut buf = [0u8; 64];
        let info = s.a.recv_from(p, &mut buf, RecvFlags::default()).unwrap();
        assert_eq!(info.from.ip(), IP_B);
        let reply = &buf[..info.len];
        assert_eq!(reply.len(), req.len());
        assert_eq!(reply[0], icmpv4::ECHO_REPLY);
        assert_eq!(u16::from_be_bytes([reply[4], reply[5]]), s.a.local_addr(p).unwrap().port(), "identifier");
        assert_eq!(u16::from_be_bytes([reply[6], reply[7]]), seq);
        assert_eq!(&reply[8..], b"0123456789abcdef");
    }
    // Only echo requests may be sent.
    let mut bad = echo_request(1, b"");
    bad[0] = 13;
    assert_eq!(s.a.send_to(p, &bad, Some(SocketAddr::new(IP_B.into(), 0))), Err(Errno::EINVAL));
    assert_eq!(s.a.send_to(p, &[8, 0, 0], Some(SocketAddr::new(IP_B.into(), 0))), Err(Errno::EINVAL));
}

#[test]
fn ping_loopback_and_self() {
    let mut s = Sim::new(51);
    let p = s.a.socket(Domain::Inet, SockType::Datagram, Some(Protocol::Icmp)).unwrap();
    for dst in [Ipv4Addr::LOCALHOST, IP_A] {
        s.a.send_to(p, &echo_request(9, b"me"), Some(SocketAddr::new(dst.into(), 0))).unwrap();
        s.run_for(Duration::from_millis(10));
        let mut buf = [0u8; 64];
        let info = s.a.recv_from(p, &mut buf, RecvFlags::default()).unwrap();
        assert_eq!(info.from.ip(), dst);
        assert_eq!(buf[0], icmpv4::ECHO_REPLY);
    }
    assert_eq!(s.frames, [0, 0]);
    // Request and reply, twice, all through the loopback queue.
    assert_eq!(s.a.stats().loop_packets, 4);
}

#[test]
fn no_reply_when_disabled_or_broadcast() {
    let mut cb = crate::sim::config(IP_B, 2);
    cb.reply_to_ping = false;
    let mut s = Sim::with_configs(52, crate::sim::config(IP_A, 1), cb, 1500);
    let p = s.a.socket(Domain::Inet, SockType::Datagram, Some(Protocol::Icmp)).unwrap();
    s.a.send_to(p, &echo_request(1, b"x"), Some(SocketAddr::new(IP_B.into(), 0))).unwrap();
    s.run_for(Duration::from_millis(100));
    assert!(!s.a.readiness(p).unwrap().contains(Readiness::READABLE));
    // A replies to unicast, but ignores pings sent to broadcast.
    let req = {
        let mut m = echo_request(1, b"bcast");
        icmpv4::fix_checksum(&mut m);
        m
    };
    let mut h = ipv4::Ipv4Header::new(IP_B, Ipv4Addr::BROADCAST, proto::ICMP, 64, req.len());
    h.ident = 1;
    s.log = Some(Vec::new());
    inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_IPV4, &ipv4_packet(&h, &req)));
    s.run_for(Duration::from_millis(100));
    assert!(icmp_in_log(s.log.as_ref().unwrap(), Dir::AtoB).is_empty());
}

#[test]
fn protocol_unreachable() {
    let mut s = Sim::new(53);
    s.log = Some(Vec::new());
    let h = ipv4::Ipv4Header::new(IP_B, IP_A, 99, 64, 12);
    inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_IPV4, &ipv4_packet(&h, &[1; 12])));
    s.run_for(Duration::from_millis(100));
    let msgs = icmp_in_log(s.log.as_ref().unwrap(), Dir::AtoB);
    assert_eq!(msgs.len(), 1);
    let (ty, code, data) = &msgs[0];
    assert_eq!((*ty, *code), (icmpv4::DEST_UNREACHABLE, icmpv4::unreachable::PROTOCOL));
    // The quote starts with the offending header and its first data bytes.
    let (q, rest) = ipv4::parse(&data[..32]).unwrap();
    assert_eq!((q.src, q.dst, q.protocol), (IP_B, IP_A, 99));
    assert_eq!(rest, &[1; 12]);
}

#[test]
fn errors_are_rate_limited_and_never_about_errors() {
    let mut s = Sim::new(54);
    s.log = Some(Vec::new());
    // 200 datagrams to a closed port within 100 ms.
    for i in 0..200u16 {
        let d = udp_v4(IP_B, 1000 + i, IP_A, 9, b"x");
        let mut h = ipv4::Ipv4Header::new(IP_B, IP_A, proto::UDP, 64, d.len());
        h.ident = i;
        inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_IPV4, &ipv4_packet(&h, &d)));
        if i % 20 == 0 {
            s.run_for(Duration::from_millis(10));
        }
    }
    s.run_for(Duration::from_millis(10));
    let sent = icmp_in_log(s.log.as_ref().unwrap(), Dir::AtoB).len();
    // A burst of 50 plus 100 per second.
    assert!((50..=62).contains(&sent), "{sent} errors sent");
    assert!(s.a.stats().icmp_rate_limited > 100);
    // No errors about ICMP errors, or about non-initial fragments
    // (RFC 1122 3.2.2). An error quoting a datagram nobody sent:
    s.run_for(Duration::from_secs(10));
    s.log = Some(Vec::new());
    let mut err = vec![icmpv4::DEST_UNREACHABLE, 3, 0, 0, 0, 0, 0, 0];
    err.extend_from_slice(&[0x45; 28]);
    icmpv4::fix_checksum(&mut err);
    let h = ipv4::Ipv4Header::new(IP_B, IP_A, proto::ICMP, 64, err.len());
    inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_IPV4, &ipv4_packet(&h, &err)));
    // A fragment (offset 8) of a datagram to a closed port; it is never
    // completed, and is not the first fragment.
    let mut h = ipv4::Ipv4Header::new(IP_B, IP_A, proto::UDP, 64, 16);
    h.frag_offset = 8;
    inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_IPV4, &ipv4_packet(&h, &[0; 16])));
    s.run_for(Duration::from_millis(100));
    assert!(icmp_in_log(s.log.as_ref().unwrap(), Dir::AtoB).is_empty());
}

#[test]
fn fragmentation_needed_lowers_the_tcp_segment_size() {
    let mut s = Sim::new(55);
    let (c, _l, a) = s.connect(8000);
    assert_eq!(s.a.tcp_info(c).unwrap().send_mss, 1448);
    // A "router" between A and B with an MTU of 1000: it drops bigger
    // packets and answers with ICMP fragmentation needed (RFC 1191).
    let pending = alloc::rc::Rc::new(core::cell::RefCell::new(Vec::<Vec<u8>>::new()));
    let p2 = pending.clone();
    s.filter = Some(alloc::boxed::Box::new(move |d, f| {
        if d == Dir::AtoB && f.len() > 1000 + 14 {
            let ip = &f[14..];
            let mut m = vec![icmpv4::DEST_UNREACHABLE, icmpv4::unreachable::FRAG_NEEDED, 0, 0, 0, 0, 0x03, 0xe8];
            m.extend_from_slice(&ip[..28]);
            icmpv4::fix_checksum(&mut m);
            let h = ipv4::Ipv4Header::new(IP_B, IP_A, proto::ICMP, 64, m.len());
            p2.borrow_mut().push(frame(Dir::BtoA, ethernet::ETHERTYPE_IPV4, &ipv4_packet(&h, &m)));
            return false;
        }
        true
    }));
    let data = pattern(300_000, 1);
    let mut sent = 0;
    let mut got = Vec::new();
    let mut buf = vec![0u8; 8192];
    assert!(s.run_until(Duration::from_secs(120), |s| {
        for f in pending.borrow_mut().drain(..) {
            s.da.inbox.push_back(f);
        }
        if sent < data.len()
            && let Ok(n) = s.a.send(c, &data[sent..])
        {
            sent += n;
        }
        while let Ok(n) = s.b.recv(a, &mut buf) {
            got.extend_from_slice(&buf[..n]);
        }
        got.len() == data.len()
    }));
    assert!(got == data);
    assert_eq!(s.a.tcp_info(c).unwrap().send_mss, 1000 - 40 - 12);
}
