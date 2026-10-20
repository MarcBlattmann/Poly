//! Hostile input: random frames, random headers with valid checksums, and
//! mutations of real traffic. Nothing may panic, and the stack must still
//! work afterwards.

use super::{fix_checksums, frame, inject, pattern};
use crate::sim::{Dir, IP_B, MAC_A, Rng, Sim};
use crate::wire::ethernet;
use crate::{Domain, Protocol, SockType};
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;

/// A sim with something listening on every protocol and an open connection.
fn busy_sim(seed: u64) -> (Sim, crate::SocketHandle, crate::SocketHandle) {
    let mut s = Sim::new(seed);
    let (c, _l, a) = s.connect(80);
    for (st, d) in [(&mut s.a, Domain::Inet), (&mut s.b, Domain::Inet6)] {
        let u = st.socket(d, SockType::Datagram, None).unwrap();
        st.bind(u, d.any(53)).unwrap();
        let p = st.socket(d, SockType::Datagram, Some(Protocol::Icmp)).unwrap();
        st.bind(p, d.any(0)).unwrap();
        let l = st.socket(d, SockType::Stream, None).unwrap();
        st.bind(l, d.any(22)).unwrap();
        st.listen(l, 4).unwrap();
    }
    (s, c, a)
}

/// The stack still carries data both ways after the abuse.
fn still_works(s: &mut Sim, c: crate::SocketHandle, a: crate::SocketHandle) {
    s.filter = None;
    let data = pattern(50_000, 99);
    assert!(s.transfer(true, c, a, &data, Duration::from_secs(600)) == data, "connection survived");
    let (c2, _l2, a2) = s.connect(81);
    assert!(s.transfer(false, a2, c2, &data, Duration::from_secs(600)) == data, "new connections work");
}

#[test]
fn random_garbage_frames() {
    let (mut s, c, a) = busy_sim(90);
    let mut r = Rng::new(90);
    for i in 0..20_000 {
        let len = r.below(1600) as usize;
        let mut f = r.bytes(len);
        if len >= 14 && r.chance(700) {
            f[..6].copy_from_slice(&MAC_A.0);
            f[6] &= 0xfe;
        }
        inject(&mut s, Dir::BtoA, f);
        if i % 100 == 0 {
            s.step();
        }
    }
    s.run_for(Duration::from_millis(10));
    assert!(s.a.stats().rx_malformed + s.a.stats().rx_ignored > 10_000);
    still_works(&mut s, c, a);
}

#[test]
fn random_headers_with_valid_checksums() {
    let (mut s, c, a) = busy_sim(91);
    let mut r = Rng::new(91);
    let mut addrs = Vec::new();
    for st in [&s.a, &s.b] {
        addrs.push(st.ipv6_link_local().unwrap().octets().to_vec());
    }
    for i in 0..30_000 {
        let ethertype = [ethernet::ETHERTYPE_IPV4, ethernet::ETHERTYPE_IPV6, ethernet::ETHERTYPE_ARP][i % 3];
        let len = r.below(200) as usize + 20;
        let mut p = r.bytes(len);
        match ethertype {
            ethernet::ETHERTYPE_IPV4 => {
                p[0] = 0x45;
                p[2..4].copy_from_slice(&(len as u16).to_be_bytes());
                p[9] = [1, 6, 17, 99][r.below(4) as usize];
                p[12..16].copy_from_slice(&IP_B.octets());
                p[16..20].copy_from_slice(&crate::sim::IP_A.octets());
                if r.chance(300) {
                    p[6] = 0x20 | (r.next() as u8 & 0x1f); // fragments
                } else {
                    p[6] = 0x40;
                    p[7] = 0;
                }
            }
            ethernet::ETHERTYPE_IPV6 if len >= 40 => {
                p[0] = 0x60;
                p[4..6].copy_from_slice(&((len - 40) as u16).to_be_bytes());
                p[6] = [0, 6, 17, 58, 43, 44, 60][r.below(7) as usize];
                p[8..24].copy_from_slice(&addrs[1]);
                p[24..40].copy_from_slice(&addrs[0]);
            }
            ethernet::ETHERTYPE_ARP if len >= 28 => {
                p[..8].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1 + (r.next() as u8 & 1)]);
            }
            _ => {}
        }
        let mut f = frame(Dir::BtoA, ethertype, &p);
        fix_checksums(&mut f);
        inject(&mut s, Dir::BtoA, f);
        if i % 100 == 0 {
            s.step();
        }
    }
    s.run_for(Duration::from_secs(40));
    still_works(&mut s, c, a);
}

#[test]
fn mutated_real_traffic() {
    // Record a real exchange, then replay mangled copies of it.
    let (mut s, c, a) = busy_sim(92);
    s.log = Some(Vec::new());
    let data = pattern(20_000, 5);
    assert!(s.transfer(true, c, a, &data, Duration::from_secs(60)) == data);
    let (_c2, _l, _a2) = s.connect(82);
    let p = s.a.socket(Domain::Inet6, SockType::Datagram, Some(Protocol::Icmp)).unwrap();
    let mut req = alloc::vec![128u8, 0, 0, 0, 0, 0, 0, 1];
    req.extend_from_slice(&[7; 20]);
    let b_ll = s.b.ipv6_link_local().unwrap();
    s.a.send_to(p, &req, Some(SocketAddr::new(b_ll.into(), 0))).unwrap();
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.send_to(u, b"datagram", Some(SocketAddr::new(IP_B.into(), 53))).unwrap();
    s.run_for(Duration::from_secs(1));
    let log = s.log.take().unwrap();
    assert!(log.len() > 20);
    let mut r = Rng::new(92);
    for i in 0..30_000 {
        let (dir, orig) = &log[r.below(log.len() as u64) as usize];
        let mut f = orig.clone();
        let n = 1 + r.below(4);
        for _ in 0..n {
            if f.len() <= 14 {
                break;
            }
            match r.below(5) {
                0 => {
                    let k = r.below(f.len() as u64) as usize;
                    f[k] ^= 1 << r.below(8);
                }
                1 => {
                    // Header area, where the interesting fields are.
                    let k = 14 + r.below(60.min(f.len() as u64 - 14)) as usize;
                    f[k] = r.next() as u8;
                }
                2 => f.truncate(14 + r.below(f.len() as u64 - 13) as usize),
                3 => {
                    let n = r.below(64) as usize;
                    f.extend(r.bytes(n));
                }
                _ => {
                    let k = 14 + r.below(f.len() as u64 - 14) as usize;
                    f[k] = [0, 0xff, 0x7f, 0x80][r.below(4) as usize];
                }
            }
        }
        if r.chance(800) {
            fix_checksums(&mut f);
        }
        inject(&mut s, *dir, f);
        if i % 50 == 0 {
            s.step();
        }
    }
    s.run_for(Duration::from_secs(120));
    // The mangled copies may legitimately have reset the connections in
    // the log; the stack itself must be fine.
    let (c3, _l3, a3) = s.connect(83);
    let data = pattern(100_000, 6);
    assert!(s.transfer(true, c3, a3, &data, Duration::from_secs(60)) == data);
}

#[test]
fn hostile_clock_does_not_panic() {
    let (mut s, c, a) = busy_sim(93);
    s.a.send(c, &[1; 50_000]).unwrap();
    s.b.send(a, &[2; 50_000]).unwrap();
    s.a.set_keepalive(c, true).unwrap();
    // Backwards, standing still, huge jumps, the end of time.
    let times = [5u64, 0, 1_000_000_000, 1_000_000_000, 3, u64::MAX / 2, u64::MAX - 1, u64::MAX, 0];
    for &ms in &times {
        let now = crate::Instant::from_millis(ms);
        s.a.poll(&mut s.da, now);
        s.b.poll(&mut s.db, now);
        let da: Vec<_> = s.da.outbox.drain(..).collect();
        let db: Vec<_> = s.db.outbox.drain(..).collect();
        s.db.inbox.extend(da);
        s.da.inbox.extend(db);
    }
    // The stack's clock never went backwards.
    assert_eq!(s.a.now(), crate::Instant::from_millis(u64::MAX));
}
