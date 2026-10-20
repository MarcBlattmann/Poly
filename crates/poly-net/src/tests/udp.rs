//! UDP: echo, connected sockets, errors, checksums, limits.

use super::{frame, inject, ipv4_packet, udp_v4};
use crate::sim::{Dir, IP_A, IP_B, Sim};
use crate::wire::{ethernet, ipv4, proto};
use crate::{Domain, Errno, Readiness, RecvFlags, Shutdown, SockType};
use alloc::vec;
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;

fn sa(ip: Ipv4Addr, port: u16) -> SocketAddr {
    SocketAddr::new(ip.into(), port)
}

#[test]
fn echo_between_stacks() {
    let mut s = Sim::new(40);
    let srv = s.b.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.b.bind(srv, sa(Ipv4Addr::UNSPECIFIED, 7)).unwrap();
    let cli = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    assert!(s.a.readiness(cli).unwrap().contains(Readiness::WRITABLE));
    for i in 0..10u8 {
        let msg = vec![i; 100 + usize::from(i)];
        assert_eq!(s.a.send_to(cli, &msg, Some(sa(IP_B, 7))), Ok(msg.len()));
        let mut buf = [0u8; 2048];
        let mut from = None;
        assert!(s.run_until(Duration::from_secs(1), |s| {
            if let Ok(info) = s.b.recv_from(srv, &mut buf, RecvFlags::default()) {
                s.b.send_to(srv, &buf[..info.len], Some(info.from)).unwrap();
                from = Some(info.from);
            }
            from.is_some()
        }));
        assert_eq!(from.unwrap().ip(), IP_A);
        assert_eq!(from.unwrap().port(), s.a.local_addr(cli).unwrap().port(), "auto-bound port");
        let mut got = None;
        assert!(s.run_until(Duration::from_secs(1), |s| {
            got = s.a.recv_from(cli, &mut buf, RecvFlags::default()).ok();
            got.is_some()
        }));
        let info = got.unwrap();
        assert_eq!(info.from, sa(IP_B, 7));
        assert_eq!(&buf[..info.len], &msg[..]);
    }
}

#[test]
fn connected_socket_filters_and_reports_port_unreachable() {
    let mut s = Sim::new(41);
    let a1 = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.connect(a1, sa(IP_B, 9)).unwrap();
    assert_eq!(s.a.local_addr(a1).unwrap().ip(), IP_A, "connect picks the source address");
    assert_eq!(s.a.peer_addr(a1), Ok(sa(IP_B, 9)));
    // Nothing listens on B:9: B answers with ICMP port unreachable.
    s.a.send(a1, b"knock").unwrap();
    assert!(s.run_until(Duration::from_secs(1), |s| s.a.readiness(a1).unwrap().contains(Readiness::ERROR)));
    assert_eq!(s.b.stats().icmp_out, 1);
    assert_eq!(s.a.send(a1, b"again"), Err(Errno::ECONNREFUSED));
    assert_eq!(s.a.send(a1, b"again"), Ok(5), "the error is reported once");
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.a.take_error(a1), Ok(Some(Errno::ECONNREFUSED)), "and again for the second datagram");
    // An unconnected socket hears nothing about it.
    let a2 = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.send_to(a2, b"knock", Some(sa(IP_B, 9))).unwrap();
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.a.take_error(a2), Ok(None));
    // A connected socket only takes datagrams from its peer.
    let b1 = s.b.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.b.bind(b1, sa(IP_B, 9)).unwrap();
    let b2 = s.b.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.b.bind(b2, sa(IP_B, 10)).unwrap();
    let a1_addr = s.a.local_addr(a1).unwrap();
    s.b.send_to(b2, b"stranger", Some(a1_addr)).unwrap();
    s.b.send_to(b1, b"peer", Some(a1_addr)).unwrap();
    s.run_for(Duration::from_millis(100));
    let mut buf = [0u8; 16];
    assert_eq!(s.a.recv(a1, &mut buf), Ok(4));
    assert_eq!(&buf[..4], b"peer");
    assert_eq!(s.a.recv(a1, &mut buf), Err(Errno::EAGAIN));
    // Disconnecting lifts the filter.
    s.a.disconnect(a1).unwrap();
    s.b.send_to(b2, b"stranger", Some(a1_addr)).unwrap();
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.a.recv(a1, &mut buf), Ok(8));
}

#[test]
fn send_errors() {
    let mut s = Sim::new(42);
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    assert_eq!(s.a.send(u, b"x"), Err(Errno::EDESTADDRREQ));
    assert_eq!(s.a.send_to(u, &[0; 1473], Some(sa(IP_B, 9))), Err(Errno::EMSGSIZE));
    assert_eq!(s.a.send_to(u, &[0; 1472], Some(sa(IP_B, 9))), Ok(1472));
    assert_eq!(s.a.send_to(u, b"x", Some(sa(Ipv4Addr::BROADCAST, 9))), Err(Errno::EACCES));
    assert_eq!(s.a.send_to(u, b"x", Some(sa(Ipv4Addr::new(10, 0, 0, 255), 9))), Err(Errno::EACCES));
    assert_eq!(s.a.send_to(u, b"x", Some(sa(Ipv4Addr::new(192, 168, 1, 1), 9))), Err(Errno::ENETUNREACH));
    assert_eq!(s.a.send_to(u, b"x", Some("[::1]:9".parse().unwrap())), Err(Errno::EAFNOSUPPORT));
    let mut buf = [0u8; 4];
    assert_eq!(s.a.recv(u, &mut buf), Err(Errno::EAGAIN));
    assert_eq!(s.a.shutdown(u, Shutdown::Both), Err(Errno::ENOTCONN));
    assert_eq!(s.a.set_nodelay(u, true), Err(Errno::ENOPROTOOPT));
    // The send queue is bounded.
    s.a.set_buffer_sizes(u, Some(2800), None).unwrap();
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.a.send_to(u, &[0; 1400], Some(sa(IP_B, 9))), Ok(1400));
    assert_eq!(s.a.send_to(u, &[0; 1400], Some(sa(IP_B, 9))), Ok(1400));
    assert_eq!(s.a.send_to(u, &[0; 1400], Some(sa(IP_B, 9))), Err(Errno::EAGAIN));
    assert!(!s.a.readiness(u).unwrap().contains(Readiness::WRITABLE));
    s.step();
    assert!(s.a.readiness(u).unwrap().contains(Readiness::WRITABLE));
}

#[test]
fn broadcast_with_so_broadcast() {
    let mut s = Sim::new(43);
    let srv = s.b.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.b.bind(srv, sa(Ipv4Addr::UNSPECIFIED, 67)).unwrap();
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.set_broadcast(u, true).unwrap();
    s.a.send_to(u, b"anyone", Some(sa(Ipv4Addr::BROADCAST, 67))).unwrap();
    s.a.send_to(u, b"subnet", Some(sa(Ipv4Addr::new(10, 0, 0, 255), 67))).unwrap();
    s.run_for(Duration::from_millis(100));
    let mut buf = [0u8; 16];
    assert_eq!(s.b.recv(srv, &mut buf), Ok(6));
    assert_eq!(&buf[..6], b"anyone");
    assert_eq!(s.b.recv(srv, &mut buf), Ok(6));
    assert_eq!(s.a.stats().arp_requests_out, 0, "broadcasts need no resolution");
    // Broadcasts to a closed port draw no ICMP error.
    s.a.send_to(u, b"x", Some(sa(Ipv4Addr::BROADCAST, 68))).unwrap();
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.b.stats().icmp_out, 0);
}

#[test]
fn checksums_are_checked() {
    let mut s = Sim::new(44);
    let srv = s.b.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.b.bind(srv, sa(IP_B, 5000)).unwrap();
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.send_to(u, b"prime arp", Some(sa(IP_B, 5000))).unwrap();
    s.run_for(Duration::from_millis(100));
    let mut buf = [0u8; 64];
    assert_eq!(s.b.recv(srv, &mut buf), Ok(9));
    // A datagram with a broken checksum is dropped.
    let mut d = udp_v4(IP_A, 1000, IP_B, 5000, b"payload");
    d[9] ^= 0x20;
    let p = ipv4_packet(&ipv4::Ipv4Header::new(IP_A, IP_B, proto::UDP, 64, d.len()), &d);
    inject(&mut s, Dir::AtoB, frame(Dir::AtoB, ethernet::ETHERTYPE_IPV4, &p));
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.b.recv(srv, &mut buf), Err(Errno::EAGAIN));
    assert_eq!(s.b.stats().rx_checksum, 1);
    // Over IPv4 a zero checksum means none was sent: accepted.
    let mut d = udp_v4(IP_A, 1000, IP_B, 5000, b"nocsum");
    d[6] = 0;
    d[7] = 0;
    let p = ipv4_packet(&ipv4::Ipv4Header::new(IP_A, IP_B, proto::UDP, 64, d.len()), &d);
    inject(&mut s, Dir::AtoB, frame(Dir::AtoB, ethernet::ETHERTYPE_IPV4, &p));
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.b.recv(srv, &mut buf), Ok(6));
    // A broken IP header checksum is caught before UDP is looked at.
    let d = udp_v4(IP_A, 1000, IP_B, 5000, b"ok");
    let mut p = ipv4_packet(&ipv4::Ipv4Header::new(IP_A, IP_B, proto::UDP, 64, d.len()), &d);
    p[8] = 1; // TTL changed without fixing the checksum
    inject(&mut s, Dir::AtoB, frame(Dir::AtoB, ethernet::ETHERTYPE_IPV4, &p));
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.b.stats().rx_checksum, 2);
    // Random corruption on the link: nothing corrupted is ever delivered.
    s.link.corrupt = 300;
    let mut delivered = 0;
    for i in 0..300u32 {
        let msg = i.to_be_bytes().repeat(50);
        s.a.send_to(u, &msg, Some(sa(IP_B, 5000))).unwrap();
        s.run_for(Duration::from_millis(10));
        let mut big = [0u8; 400];
        while let Ok(n) = s.b.recv(srv, &mut big) {
            assert_eq!(&big[..n], &msg[..], "a corrupted datagram got through");
            delivered += 1;
        }
    }
    assert!(delivered > 150 && delivered < 300, "{delivered}");
}

#[test]
fn receive_queue_is_bounded_and_peek_and_truncation_work() {
    let mut s = Sim::new(45);
    let srv = s.b.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.b.bind(srv, sa(IP_B, 5001)).unwrap();
    s.b.set_buffer_sizes(srv, None, Some(1000)).unwrap();
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    for _ in 0..5 {
        s.a.send_to(u, &[7; 300], Some(sa(IP_B, 5001))).unwrap();
    }
    s.run_for(Duration::from_millis(100));
    let mut small = [0u8; 10];
    let info = s.b.recv_from(srv, &mut small, RecvFlags { peek: true }).unwrap();
    assert_eq!((info.len, info.full_len), (10, 300));
    let mut n = 0;
    while let Ok(info) = s.b.recv_from(srv, &mut small, RecvFlags::default()) {
        assert_eq!(info.full_len, 300);
        n += 1;
    }
    assert_eq!(n, 3, "only what fits in 1000 bytes is kept");
}

#[test]
fn shutdown_of_connected_datagram_socket() {
    let mut s = Sim::new(46);
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.connect(u, sa(IP_B, 9)).unwrap();
    s.a.shutdown(u, Shutdown::Write).unwrap();
    assert_eq!(s.a.send(u, b"x"), Err(Errno::EPIPE));
    s.a.shutdown(u, Shutdown::Read).unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(s.a.recv(u, &mut buf), Ok(0));
    assert!(s.a.readiness(u).unwrap().contains(Readiness::HANGUP | Readiness::READABLE));
}

#[test]
fn loopback_udp() {
    let mut s = Sim::new(47);
    let srv = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.bind(srv, sa(Ipv4Addr::LOCALHOST, 53)).unwrap();
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.send_to(u, b"query", Some(sa(Ipv4Addr::LOCALHOST, 53))).unwrap();
    s.a.send_to(u, b"refused", Some(sa(Ipv4Addr::LOCALHOST, 54))).unwrap();
    s.run_for(Duration::from_millis(10));
    let mut buf = [0u8; 16];
    let info = s.a.recv_from(srv, &mut buf, RecvFlags::default()).unwrap();
    assert_eq!(&buf[..info.len], b"query");
    assert_eq!(info.from.ip(), Ipv4Addr::LOCALHOST);
    assert_eq!(s.a.stats().icmp_out, 1, "port unreachable for 54, delivered locally");
    assert_eq!(s.frames, [0, 0]);
}
