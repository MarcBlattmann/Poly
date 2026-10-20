//! IPv6: link-local addressing, Neighbor Discovery, ICMPv6 echo, TCP and
//! UDP over IPv6.

use super::{fix_checksums, frame, inject, pattern};
use crate::sim::{Dir, LinkConfig, MAC_A, MAC_B, Sim};
use crate::wire::{ethernet, icmpv6, ipv6, proto};
use crate::{Domain, Errno, Protocol, Readiness, RecvFlags, SockType, TcpState};
use alloc::vec;
use alloc::vec::Vec;
use core::net::{Ipv6Addr, SocketAddr};
use core::time::Duration;

fn lls(s: &Sim) -> (Ipv6Addr, Ipv6Addr) {
    (s.a.ipv6_link_local().unwrap(), s.b.ipv6_link_local().unwrap())
}

#[test]
fn link_local_addresses_come_from_the_mac() {
    let s = Sim::new(80);
    let (a, b) = lls(&s);
    assert_eq!(a, "fe80::ff:fe00:a".parse::<Ipv6Addr>().unwrap());
    assert_eq!(b, "fe80::ff:fe00:b".parse::<Ipv6Addr>().unwrap());
}

#[test]
fn ping6_resolves_with_ndp() {
    let mut s = Sim::new(81);
    let (_, b) = lls(&s);
    let p = s.a.socket(Domain::Inet6, SockType::Datagram, Some(Protocol::Icmp)).unwrap();
    let mut req = vec![icmpv6::ECHO_REQUEST, 0, 0, 0, 0, 0, 0, 1];
    req.extend_from_slice(b"hello six");
    s.a.send_to(p, &req, Some(SocketAddr::new(b.into(), 0))).unwrap();
    assert!(s.run_until(Duration::from_secs(1), |s| s.a.readiness(p).unwrap().contains(Readiness::READABLE)));
    let mut buf = [0u8; 64];
    let info = s.a.recv_from(p, &mut buf, RecvFlags::default()).unwrap();
    assert_eq!(info.from.ip(), b);
    assert_eq!(buf[0], icmpv6::ECHO_REPLY);
    assert_eq!(&buf[8..info.len], b"hello six");
    assert_eq!(s.a.neighbor(b.into()), Some(MAC_B));
    assert_eq!(s.b.neighbor(lls(&s).0.into()), Some(MAC_A), "learned from the solicitation");
    assert_eq!(s.a.stats().ndp_solicits_out, 1);
    assert_eq!(s.b.stats().ndp_adverts_out, 1);
    // An IPv4 ping type on an IPv6 socket is refused.
    let mut bad = req.clone();
    bad[0] = 8;
    assert_eq!(s.a.send_to(p, &bad, Some(SocketAddr::new(b.into(), 0))), Err(Errno::EINVAL));
}

#[test]
fn tcp_over_ipv6_with_loss() {
    let mut s = Sim::new(82);
    s.link = LinkConfig {
        loss: 20,
        duplicate: 5,
        corrupt: 5,
        delay: Duration::from_millis(10),
        jitter: Duration::from_millis(5),
    };
    let (a_ll, b_ll) = lls(&s);
    let l = s.b.socket(Domain::Inet6, SockType::Stream, None).unwrap();
    s.b.bind(l, SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 443)).unwrap();
    s.b.listen(l, 4).unwrap();
    let c = s.a.socket(Domain::Inet6, SockType::Stream, None).unwrap();
    assert_eq!(s.a.connect(c, SocketAddr::new(b_ll.into(), 443)), Err(Errno::EINPROGRESS));
    let mut acc = None;
    assert!(s.run_until(Duration::from_secs(60), |s| {
        acc = acc.or(s.b.accept(l).ok());
        acc.is_some() && s.a.tcp_state(c) == Ok(TcpState::Established)
    }));
    let (srv, peer) = acc.unwrap();
    assert_eq!(peer.ip(), a_ll);
    assert_eq!(s.a.tcp_info(c).unwrap().send_mss, 1500 - 60 - 12);
    let data = pattern(1 << 20, 3);
    assert!(s.transfer(true, c, srv, &data, Duration::from_secs(600)) == data);
}

#[test]
fn udp_over_ipv6_and_port_unreachable() {
    let mut s = Sim::new(83);
    let (a_ll, b_ll) = lls(&s);
    let srv = s.b.socket(Domain::Inet6, SockType::Datagram, None).unwrap();
    s.b.bind(srv, SocketAddr::new(b_ll.into(), 53)).unwrap();
    let u = s.a.socket(Domain::Inet6, SockType::Datagram, None).unwrap();
    s.a.send_to(u, b"v6 query", Some(SocketAddr::new(b_ll.into(), 53))).unwrap();
    s.run_for(Duration::from_millis(100));
    let mut buf = [0u8; 32];
    let info = s.b.recv_from(srv, &mut buf, RecvFlags::default()).unwrap();
    assert_eq!(&buf[..info.len], b"v6 query");
    assert_eq!(info.from.ip(), a_ll);
    // Connected to a closed port: ICMPv6 port unreachable.
    let c = s.a.socket(Domain::Inet6, SockType::Datagram, None).unwrap();
    s.a.connect(c, SocketAddr::new(b_ll.into(), 54)).unwrap();
    s.a.send(c, b"x").unwrap();
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::ECONNREFUSED)));
    // An IPv4 socket does not see IPv6 traffic.
    let v4 = s.b.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.b.bind(v4, SocketAddr::new(core::net::Ipv4Addr::UNSPECIFIED.into(), 55)).unwrap();
    s.a.send_to(u, b"nope", Some(SocketAddr::new(b_ll.into(), 55))).unwrap();
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.b.recv(v4, &mut buf), Err(Errno::EAGAIN));
}

#[test]
fn loopback_v6() {
    let mut s = Sim::new(84);
    let l = s.a.socket(Domain::Inet6, SockType::Stream, None).unwrap();
    s.a.bind(l, "[::1]:8080".parse().unwrap()).unwrap();
    s.a.listen(l, 1).unwrap();
    let c = s.a.socket(Domain::Inet6, SockType::Stream, None).unwrap();
    assert_eq!(s.a.connect(c, "[::1]:8080".parse().unwrap()), Err(Errno::EINPROGRESS));
    let mut acc = None;
    assert!(s.run_until(Duration::from_secs(1), |s| {
        acc = acc.or(s.a.accept(l).ok());
        acc.is_some()
    }));
    let data = pattern(100_000, 4);
    assert!(s.transfer_local(c, acc.unwrap().0, &data) == data);
    assert_eq!(s.frames, [0, 0]);
}

/// A Neighbor Solicitation from B for `target`, with the given hop limit.
fn solicitation(s: &Sim, target: Ipv6Addr, hop_limit: u8) -> Vec<u8> {
    let b = s.b.ipv6_link_local().unwrap();
    let dst = ipv6::solicited_node(target);
    let mut body = [0u8; 32];
    let len = icmpv6::write_neighbor(&mut body, target, icmpv6::OPT_SOURCE_LLADDR, MAC_B);
    body[0] = icmpv6::NEIGHBOR_SOLICIT;
    let h = ipv6::Ipv6Header {
        traffic_class: 0,
        flow_label: 0,
        payload_len: len,
        next_header: proto::ICMPV6,
        hop_limit,
        src: b,
        dst,
    };
    let mut p = vec![0u8; 40 + len];
    ipv6::emit(&mut p, &h);
    p[40..].copy_from_slice(&body[..len]);
    let mut f = frame(Dir::BtoA, ethernet::ETHERTYPE_IPV6, &p);
    f[..6].copy_from_slice(&crate::MacAddr::ipv6_multicast(dst).0);
    fix_checksums(&mut f);
    f
}

#[test]
fn neighbor_discovery_validation() {
    let mut s = Sim::new(85);
    let (a, _) = lls(&s);
    // Hop limit must be 255 (RFC 4861 7.1.1): a forwarded NS is ignored.
    let f = solicitation(&s, a, 64);
    inject(&mut s, Dir::BtoA, f);
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.a.stats().ndp_adverts_out, 0);
    // A solicitation for someone else's address is ignored.
    let f = solicitation(&s, "fe80::1234".parse().unwrap(), 255);
    inject(&mut s, Dir::BtoA, f);
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.a.stats().ndp_adverts_out, 0);
    // A proper one is answered.
    let f = solicitation(&s, a, 255);
    inject(&mut s, Dir::BtoA, f);
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.a.stats().ndp_adverts_out, 1);
    // Without IPv6, nothing.
    let mut c = crate::sim::config(crate::sim::IP_A, 1);
    c.ipv6 = false;
    let mut s2 = Sim::with_configs(86, c, crate::sim::config(crate::sim::IP_B, 2), 1500);
    let f = solicitation(&s2, a, 255);
    inject(&mut s2, Dir::BtoA, f);
    s2.run_for(Duration::from_millis(10));
    assert_eq!(s2.a.stats().ndp_adverts_out, 0);
    assert_eq!(s2.a.ipv6_link_local(), None);
}

#[test]
fn global_address_and_gateway() {
    let mut ca = crate::sim::config(crate::sim::IP_A, 1);
    let mut cb = crate::sim::config(crate::sim::IP_B, 2);
    ca.ipv6_global = Some(crate::Ipv6Cidr::new("2001:db8::a".parse().unwrap(), 64));
    cb.ipv6_global = Some(crate::Ipv6Cidr::new("2001:db8::b".parse().unwrap(), 64));
    let mut s = Sim::with_configs(87, ca, cb, 1500);
    let srv = s.b.socket(Domain::Inet6, SockType::Datagram, None).unwrap();
    s.b.bind(srv, "[2001:db8::b]:9".parse().unwrap()).unwrap();
    let u = s.a.socket(Domain::Inet6, SockType::Datagram, None).unwrap();
    s.a.send_to(u, b"global", Some("[2001:db8::b]:9".parse().unwrap())).unwrap();
    s.run_for(Duration::from_millis(100));
    let mut buf = [0u8; 16];
    let info = s.b.recv_from(srv, &mut buf, RecvFlags::default()).unwrap();
    assert_eq!(info.from.ip(), "2001:db8::a".parse::<Ipv6Addr>().unwrap(), "global source for a global peer");
    // Off-prefix with no router: unreachable.
    let r = s.a.send_to(u, b"x", Some("[2001:db9::1]:9".parse().unwrap()));
    assert_eq!(r, Err(Errno::ENETUNREACH));
}
