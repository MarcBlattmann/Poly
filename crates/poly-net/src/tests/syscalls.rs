//! What the system call layers use besides the BSD-shaped calls:
//! readiness change reports, queue sizes, per-socket IP options,
//! keepalive timing and closing with a reset (ADR 0048).

use super::pattern;
use crate::sim::{Dir, IP_B, Sim};
use crate::wire::{ethernet, ipv4, tcp};
use crate::{Domain, Errno, Readiness, SockType, SocketHandle, TcpState};
use alloc::vec;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;

fn changed(s: &mut crate::Stack) -> Vec<SocketHandle> {
    let mut v = Vec::new();
    s.take_changed(&mut v);
    v
}

#[test]
fn take_changed_reports_each_change_once() {
    let mut s = Sim::new(41);
    let (c, l, a) = s.connect(80);
    // Everything is new at the first look.
    let first = changed(&mut s.b);
    assert!(first.contains(&l) && first.contains(&a));
    assert!(changed(&mut s.b).is_empty(), "nothing moved since");
    // Data arriving is a change; more data on a readable socket too.
    s.a.send(c, b"one").unwrap();
    assert!(s.run_until(Duration::from_secs(5), |s| s.b.bytes_readable(a) == Ok(3)));
    assert_eq!(changed(&mut s.b), vec![a]);
    s.a.send(c, b"two").unwrap();
    assert!(s.run_until(Duration::from_secs(5), |s| s.b.bytes_readable(a) == Ok(6)));
    assert!(s.b.readiness(a).unwrap().contains(Readiness::READABLE));
    assert_eq!(changed(&mut s.b), vec![a], "more input counts although the socket was readable");
    // Reading it all changes it again.
    let mut buf = [0u8; 16];
    assert_eq!(s.b.recv(a, &mut buf), Ok(6));
    assert_eq!(changed(&mut s.b), vec![a]);
    assert!(changed(&mut s.b).is_empty());
    // A closed socket is no longer reported.
    s.b.close(a).unwrap();
    assert!(!changed(&mut s.b).contains(&a));
}

#[test]
fn queue_sizes_for_fionread_and_siocoutq() {
    let mut s = Sim::new(42);
    let (c, l, a) = s.connect(81);
    assert_eq!(s.b.bytes_readable(l), Err(Errno::EINVAL), "tcp(7): SIOCINQ on a listener");
    assert_eq!(s.b.send_queued(l), Err(Errno::EINVAL));
    let data = pattern(5000, 1);
    assert_eq!(s.a.send(c, &data), Ok(5000));
    assert_eq!(s.a.send_queued(c), Ok(5000), "nothing acknowledged before the next poll");
    assert!(s.run_until(Duration::from_secs(5), |s| s.b.bytes_readable(a) == Ok(5000)));
    assert!(s.run_until(Duration::from_secs(5), |s| s.a.send_queued(c) == Ok(0)));
    // UDP: the size of the next datagram, 0 without one.
    let u = s.b.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.b.bind(u, SocketAddr::new(IP_B.into(), 53)).unwrap();
    assert_eq!(s.b.bytes_readable(u), Ok(0));
    let ua = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.send_to(ua, b"abc", Some(SocketAddr::new(IP_B.into(), 53))).unwrap();
    s.a.send_to(ua, b"defgh", Some(SocketAddr::new(IP_B.into(), 53))).unwrap();
    assert_eq!(s.a.send_queued(ua), Ok(8));
    assert!(s.run_until(Duration::from_secs(5), |s| s.b.bytes_readable(u) == Ok(3)));
    assert_eq!(s.b.buffer_sizes(u), Ok((64 * 1024, 128 * 1024)));
}

/// TTL and TOS of the IPv4 packets going `dir`.
fn ip_fields(log: &[(Dir, Vec<u8>)], dir: Dir, protocol: u8) -> Vec<(u8, u8)> {
    log.iter()
        .filter(|(d, _)| *d == dir)
        .filter_map(|(_, f)| {
            let e = ethernet::parse(f).ok()?;
            let (h, _) = ipv4::parse(e.payload).ok()?;
            (h.protocol == protocol).then_some((h.ttl, h.tos))
        })
        .collect()
}

#[test]
fn per_socket_ttl_and_tos_reach_the_wire() {
    let mut s = Sim::new(43);
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    assert_eq!(s.a.ip_options(u), Ok((None, 0)));
    s.a.set_ip_options(u, Some(7), 0x10).unwrap();
    s.log = Some(Vec::new());
    s.a.send_to(u, b"x", Some(SocketAddr::new(IP_B.into(), 9))).unwrap();
    s.run_for(Duration::from_millis(100));
    let log = s.log.take().unwrap();
    assert_eq!(ip_fields(&log, Dir::AtoB, crate::wire::proto::UDP), vec![(7, 0x10)]);
    // Other packets keep the stack's defaults.
    let (c, _l, _a) = s.connect(82);
    s.log = Some(Vec::new());
    s.a.send(c, b"y").unwrap();
    s.run_for(Duration::from_millis(100));
    let log = s.log.take().unwrap();
    let tcp_fields = ip_fields(&log, Dir::AtoB, crate::wire::proto::TCP);
    assert!(!tcp_fields.is_empty() && tcp_fields.iter().all(|&f| f == (s.a.default_ttl(), 0)));
}

#[test]
fn keepalive_timing_per_socket() {
    let mut s = Sim::new(44);
    let (c, _l, _a) = s.connect(83);
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    assert_eq!(s.a.set_keepalive_params(u, Some(Duration::from_secs(1)), None, None), Err(Errno::ENOPROTOOPT));
    s.a.set_keepalive_params(c, Some(Duration::from_secs(5)), Some(Duration::from_secs(1)), Some(2)).unwrap();
    assert_eq!(s.a.keepalive_params(c), Ok((Duration::from_secs(5), Duration::from_secs(1), 2)));
    s.a.set_keepalive(c, true).unwrap();
    assert!(s.a.flags(c).unwrap().keepalive);
    // The peer answers probes: nothing fails, probes went out.
    s.run_for(Duration::from_secs(20));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Established));
    assert!(s.a.tcp_info(c).unwrap().stats.keepalive_probes >= 2);
    // A dead peer: the connection fails after idle + probes.
    s.cut = true;
    assert!(s.run_until(Duration::from_secs(60), |s| s.a.readiness(c).unwrap().contains(Readiness::ERROR)));
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::ETIMEDOUT)));
}

#[test]
fn abort_sends_a_reset() {
    let mut s = Sim::new(45);
    let (c, _l, a) = s.connect(84);
    s.log = Some(Vec::new());
    s.a.abort(c).unwrap();
    assert_eq!(s.a.readiness(c), Err(Errno::EBADF), "the handle is gone");
    assert!(s.run_until(Duration::from_secs(5), |s| s.b.readiness(a).unwrap().contains(Readiness::ERROR)));
    assert_eq!(s.b.take_error(a), Ok(Some(Errno::ECONNRESET)));
    let log = s.log.take().unwrap();
    let rst = log.iter().any(|(d, f)| {
        *d == Dir::AtoB
            && ethernet::parse(f)
                .ok()
                .and_then(|e| ipv4::parse(e.payload).ok())
                .and_then(|(_, p)| tcp::parse(p).ok())
                .is_some_and(|(h, _)| h.flags & tcp::RST != 0)
    });
    assert!(rst, "a RST went out");
    s.run_for(Duration::from_secs(1));
    assert_eq!(s.a.socket_count(), 0, "nothing lingers after the reset");
}
