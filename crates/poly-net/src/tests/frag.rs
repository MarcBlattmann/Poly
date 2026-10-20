//! IPv4 fragment reassembly through the whole stack.

use super::{frame, icmp_in_log, inject, pattern, udp_v4};
use crate::reassembly::split;
use crate::sim::{Dir, IP_A, IP_B, Sim};
use crate::wire::{ethernet, icmpv4, ipv4, proto};
use crate::{Domain, RecvFlags, SockType};
use alloc::vec;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;

/// Fragments (as frames from B to A) of one UDP datagram to A:`port`.
fn fragments(ident: u16, data: &[u8], port: u16, piece: usize) -> Vec<Vec<u8>> {
    let d = udp_v4(IP_B, 4000, IP_A, port, data);
    split(&d, piece)
        .into_iter()
        .map(|(off, mf, bytes)| {
            let mut h = ipv4::Ipv4Header::new(IP_B, IP_A, proto::UDP, 64, bytes.len());
            h.ident = ident;
            h.frag_offset = off;
            h.more_fragments = mf;
            let mut p = vec![0u8; 20 + bytes.len()];
            ipv4::emit(&mut p, &h);
            p[20..].copy_from_slice(&bytes);
            frame(Dir::BtoA, ethernet::ETHERTYPE_IPV4, &p)
        })
        .collect()
}

fn socket(s: &mut Sim, port: u16) -> crate::SocketHandle {
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.bind(u, SocketAddr::new(IP_A.into(), port)).unwrap();
    u
}

fn recv_all(s: &mut Sim, u: crate::SocketHandle) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 70_000];
    while let Ok(i) = s.a.recv_from(u, &mut buf, RecvFlags::default()) {
        out.push(buf[..i.len].to_vec());
    }
    out
}

#[test]
fn reassembles_in_any_order_up_to_64k() {
    let mut s = Sim::new(60);
    let u = socket(&mut s, 7);
    let data = pattern(65_000, 1);
    let mut frags = fragments(1, &data, 7, 1480);
    let mut rng = crate::sim::Rng::new(5);
    for i in (1..frags.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        frags.swap(i, j);
    }
    for f in frags {
        inject(&mut s, Dir::BtoA, f);
    }
    s.run_for(Duration::from_millis(10));
    assert_eq!(recv_all(&mut s, u), vec![data]);
    assert_eq!(s.a.stats().reassembly.completed, 1);
}

#[test]
fn duplicates_and_overlaps() {
    let mut s = Sim::new(61);
    let u = socket(&mut s, 7);
    let data = pattern(3000, 2);
    // Duplicates and an overlapping re-send of the middle, all before the
    // last piece completes the datagram. (Duplicates arriving after that
    // would start a new datagram: IP may deliver duplicates.)
    let frags = fragments(2, &data, 7, 1000);
    let middle = fragments(2, &data, 7, 504);
    for f in [&frags[0], &frags[0], &middle[2], &frags[1], &middle[3], &frags[1], &frags[2], &frags[3]] {
        inject(&mut s, Dir::BtoA, f.clone());
    }
    s.run_for(Duration::from_millis(10));
    let got = recv_all(&mut s, u);
    assert_eq!(got.len(), 1, "delivered once");
    assert!(got[0] == data);
    // Overlapping fragments that disagree: the datagram is dropped.
    let evil = {
        let mut d = data.clone();
        d[1500] ^= 0xff;
        d
    };
    let a = fragments(3, &data, 7, 1000);
    let b = fragments(3, &evil, 7, 504);
    inject(&mut s, Dir::BtoA, a[0].clone());
    inject(&mut s, Dir::BtoA, b[2].clone());
    inject(&mut s, Dir::BtoA, a[1].clone());
    inject(&mut s, Dir::BtoA, a[2].clone());
    inject(&mut s, Dir::BtoA, a[3].clone());
    s.run_for(Duration::from_millis(10));
    assert!(recv_all(&mut s, u).is_empty());
    assert_eq!(s.a.stats().reassembly.overlaps, 1);
}

#[test]
fn timeout_sends_time_exceeded() {
    let mut s = Sim::new(62);
    let _u = socket(&mut s, 7);
    let data = pattern(3000, 3);
    let frags = fragments(4, &data, 7, 1000);
    s.log = Some(Vec::new());
    inject(&mut s, Dir::BtoA, frags[0].clone());
    inject(&mut s, Dir::BtoA, frags[2].clone());
    s.run_for(Duration::from_secs(29));
    assert!(icmp_in_log(s.log.as_ref().unwrap(), Dir::AtoB).is_empty());
    s.run_for(Duration::from_secs(2));
    let msgs = icmp_in_log(s.log.as_ref().unwrap(), Dir::AtoB);
    assert_eq!(msgs.len(), 1);
    assert_eq!((msgs[0].0, msgs[0].1), (icmpv4::TIME_EXCEEDED, icmpv4::time_exceeded::REASSEMBLY));
    // The quote is the first fragment's header and 8 bytes: the UDP header.
    let q = &msgs[0].2;
    assert_eq!(u16::from_be_bytes([q[4], q[5]]), 4, "identification");
    assert_eq!(u16::from_be_bytes([q[22], q[23]]), 7, "UDP destination port");
    // The late last piece now starts a new datagram that never completes.
    inject(&mut s, Dir::BtoA, frags[1].clone());
    s.run_for(Duration::from_secs(31));
    assert_eq!(icmp_in_log(s.log.as_ref().unwrap(), Dir::AtoB).len(), 1, "no first fragment, no error");
    assert_eq!(s.a.stats().reassembly.timed_out, 2);
}

#[test]
fn limits_hold_under_a_fragment_flood() {
    let mut s = Sim::new(63);
    let u = socket(&mut s, 7);
    // 1000 different datagrams, only first fragments: bounded state.
    for id in 0..1000u16 {
        let frags = fragments(id, &pattern(3000, u64::from(id)), 7, 1000);
        inject(&mut s, Dir::BtoA, frags[0].clone());
        if id % 50 == 0 {
            s.run_for(Duration::from_millis(1));
        }
    }
    s.run_for(Duration::from_millis(1));
    let st = s.a.stats().reassembly;
    assert!(st.dropped >= 1000 - 16, "{st:?}");
    // A good datagram still gets through once the flood has timed out.
    s.run_for(Duration::from_secs(31));
    let data = pattern(2000, 9);
    for f in fragments(5000, &data, 7, 1000) {
        inject(&mut s, Dir::BtoA, f);
    }
    s.run_for(Duration::from_millis(10));
    assert_eq!(recv_all(&mut s, u), vec![data]);
}

#[test]
fn reassembled_tcp_and_icmp_work_too() {
    let mut s = Sim::new(64);
    // A big ping to A in fragments: the echo reply would exceed the MTU,
    // so none is sent (no outgoing fragmentation).
    let mut m = vec![0u8; 3000];
    m[0] = icmpv4::ECHO_REQUEST;
    icmpv4::fix_checksum(&mut m);
    s.log = Some(Vec::new());
    for (off, mf, bytes) in split(&m, 1480) {
        let mut h = ipv4::Ipv4Header::new(IP_B, IP_A, proto::ICMP, 64, bytes.len());
        h.ident = 77;
        h.frag_offset = off;
        h.more_fragments = mf;
        let mut p = vec![0u8; 20 + bytes.len()];
        ipv4::emit(&mut p, &h);
        p[20..].copy_from_slice(&bytes);
        inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_IPV4, &p));
    }
    s.run_for(Duration::from_millis(10));
    assert_eq!(s.a.stats().reassembly.completed, 1);
    assert!(icmp_in_log(s.log.as_ref().unwrap(), Dir::AtoB).is_empty());
    // A small fragmented ping is answered.
    let mut m = vec![0u8; 40];
    m[0] = icmpv4::ECHO_REQUEST;
    icmpv4::fix_checksum(&mut m);
    for (off, mf, bytes) in split(&m, 16) {
        let mut h = ipv4::Ipv4Header::new(IP_B, IP_A, proto::ICMP, 64, bytes.len());
        h.ident = 78;
        h.frag_offset = off;
        h.more_fragments = mf;
        let mut p = vec![0u8; 20 + bytes.len()];
        ipv4::emit(&mut p, &h);
        p[20..].copy_from_slice(&bytes);
        inject(&mut s, Dir::BtoA, frame(Dir::BtoA, ethernet::ETHERTYPE_IPV4, &p));
    }
    s.run_for(Duration::from_millis(100));
    let msgs = icmp_in_log(s.log.as_ref().unwrap(), Dir::AtoB);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].0, icmpv4::ECHO_REPLY);
}
