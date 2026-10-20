//! ARP: resolution, queueing, retries, failure, expiry, routing via a gateway.

use super::{frame, inject};
use crate::sim::{Dir, IP_A, IP_B, MAC_A, MAC_B, Sim, config};
use crate::wire::{arp, ethernet};
use crate::{Domain, Errno, Ipv4Cidr, MacAddr, Readiness, SockType, TcpState};
use alloc::vec::Vec;
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;

const NOBODY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 3);

fn arps(log: &[(Dir, Vec<u8>)], dir: Dir) -> Vec<arp::ArpPacket> {
    log.iter()
        .filter(|(d, _)| *d == dir)
        .filter_map(|(_, f)| {
            let e = ethernet::parse(f).ok()?;
            (e.ethertype == ethernet::ETHERTYPE_ARP).then_some(())?;
            arp::ArpPacket::parse(e.payload).ok()
        })
        .collect()
}

#[test]
fn resolution_learns_both_ways_and_releases_queued_packets_in_order() {
    let mut s = Sim::new(70);
    s.link.delay = Duration::from_millis(50);
    s.log = Some(Vec::new());
    let srv = s.b.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.b.bind(srv, SocketAddr::new(IP_B.into(), 9)).unwrap();
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    for i in 0..3u8 {
        s.a.send_to(u, &[i], Some(SocketAddr::new(IP_B.into(), 9))).unwrap();
    }
    assert_eq!(s.a.neighbor(IP_B.into()), None);
    s.run_for(Duration::from_millis(200));
    assert_eq!(s.a.neighbor(IP_B.into()), Some(MAC_B));
    assert_eq!(s.b.neighbor(IP_A.into()), Some(MAC_A), "learned from the request");
    let log = s.log.take().unwrap();
    let req = arps(&log, Dir::AtoB);
    assert_eq!(req.len(), 1, "one request for three packets");
    assert_eq!((req[0].op, req[0].target_ip), (arp::OP_REQUEST, IP_B));
    let rep = arps(&log, Dir::BtoA);
    assert_eq!((rep[0].op, rep[0].sender_mac), (arp::OP_REPLY, MAC_B));
    let mut buf = [0u8; 4];
    for i in 0..3u8 {
        assert_eq!(s.b.recv(srv, &mut buf), Ok(1));
        assert_eq!(buf[0], i);
    }
    // B could answer without asking.
    assert_eq!(s.b.stats().arp_requests_out, 0);
}

#[test]
fn unanswered_requests_fail_with_ehostunreach() {
    let mut s = Sim::new(71);
    s.log = Some(Vec::new());
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.connect(u, SocketAddr::new(NOBODY.into(), 9)).unwrap();
    s.a.send(u, b"hello?").unwrap();
    s.run_for(Duration::from_millis(2900));
    assert_eq!(s.a.take_error(u), Ok(None));
    s.run_for(Duration::from_millis(200));
    assert_eq!(s.a.take_error(u), Ok(Some(Errno::EHOSTUNREACH)));
    let reqs = arps(s.log.as_ref().unwrap(), Dir::AtoB);
    assert_eq!(reqs.len(), 3, "the request and two retries, a second apart");
    // TCP gives up right away when the SYN cannot be delivered.
    let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    assert_eq!(s.a.connect(c, SocketAddr::new(NOBODY.into(), 80)), Err(Errno::EINPROGRESS));
    assert!(s.run_until(Duration::from_secs(10), |s| s.a.readiness(c).unwrap().contains(Readiness::ERROR)));
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::EHOSTUNREACH)));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Closed));
}

#[test]
fn entries_expire_and_are_refreshed_while_in_use() {
    let mut s = Sim::new(72);
    let (c, _l, a) = s.connect(80);
    let before = s.a.stats().arp_requests_out + s.b.stats().arp_requests_out;
    // Keep the connection busy for three minutes.
    let mut buf = [0u8; 64];
    for _ in 0..180 {
        s.a.send(c, b"tick").unwrap();
        s.run_for(Duration::from_secs(1));
        assert_eq!(s.b.recv(a, &mut buf), Ok(4));
    }
    // Either side's request refreshes both caches (RFC 826 merge).
    let refreshes = s.a.stats().arp_requests_out + s.b.stats().arp_requests_out - before;
    assert!((2..=6).contains(&refreshes), "{refreshes} refreshes in 180 s with a 60 s lifetime");
    assert!(s.a.stats().arp_requests_out > 0);
    // Idle entries disappear.
    s.run_for(Duration::from_secs(200));
    assert_eq!(s.a.neighbor(IP_B.into()), None);
}

#[test]
fn unsolicited_replies_do_not_create_entries() {
    let mut s = Sim::new(73);
    let fake = arp::ArpPacket {
        op: arp::OP_REPLY,
        sender_mac: MacAddr([2, 9, 9, 9, 9, 9]),
        sender_ip: NOBODY,
        target_mac: MAC_A,
        target_ip: Ipv4Addr::new(10, 0, 0, 77),
    };
    let mut b = [0u8; arp::LEN];
    fake.emit(&mut b);
    inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_ARP, &b));
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.a.neighbor(NOBODY.into()), None);
    // Requests for addresses that are not ours get no reply.
    let req = arp::ArpPacket { op: arp::OP_REQUEST, target_ip: Ipv4Addr::new(10, 0, 0, 77), ..fake };
    req.emit(&mut b);
    inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_ARP, &b));
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.a.stats().arp_replies_out, 0);
    // A request for us from a new neighbour: answered, and remembered.
    let req = arp::ArpPacket { op: arp::OP_REQUEST, target_ip: IP_A, ..fake };
    req.emit(&mut b);
    inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_ARP, &b));
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.a.stats().arp_replies_out, 1);
    assert_eq!(s.a.neighbor(NOBODY.into()), Some(fake.sender_mac));
}

#[test]
fn off_subnet_traffic_goes_to_the_gateway() {
    let mut ca = config(IP_A, 1);
    ca.gateway = Some(IP_B);
    let mut s = Sim::with_configs(74, ca, config(IP_B, 2), 1500);
    s.log = Some(Vec::new());
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.send_to(u, b"far away", Some(SocketAddr::new(Ipv4Addr::new(192, 0, 2, 1).into(), 9))).unwrap();
    s.run_for(Duration::from_millis(100));
    let log = s.log.take().unwrap();
    assert_eq!(arps(&log, Dir::AtoB)[0].target_ip, IP_B, "resolves the gateway");
    let delivered = log.iter().any(|(d, f)| {
        *d == Dir::AtoB
            && ethernet::parse(f).is_ok_and(|e| {
                e.dst == MAC_B
                    && e.ethertype == ethernet::ETHERTYPE_IPV4
                    && crate::wire::ipv4::parse(e.payload).is_ok_and(|(h, _)| h.dst == Ipv4Addr::new(192, 0, 2, 1))
            })
    });
    assert!(delivered);
    assert!(s.b.stats().rx_ignored > 0, "B is not a router: it drops the packet");
    // Without a gateway, the same destination is unreachable at once.
    s.a.set_ipv4(Some(Ipv4Cidr::new(IP_A, 24)), None);
    let r = s.a.send_to(u, b"x", Some(SocketAddr::new(Ipv4Addr::new(192, 0, 2, 1).into(), 9)));
    assert_eq!(r, Err(Errno::ENETUNREACH));
    let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    assert_eq!(s.a.connect(c, SocketAddr::new(Ipv4Addr::new(192, 0, 2, 1).into(), 80)), Err(Errno::ENETUNREACH));
}
