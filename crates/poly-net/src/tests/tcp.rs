//! TCP over a clean (or lightly impaired) link: handshakes, transfers,
//! teardown in every order, resets, windows, Nagle, delayed ACKs, timers.

use super::pattern;
use crate::sim::{Dir, IP_A, IP_B, Sim};
use crate::wire::{ethernet, ipv4, tcp};
use crate::{Domain, Errno, Readiness, Shutdown, SockType, TcpState};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;

fn b_addr(port: u16) -> SocketAddr {
    SocketAddr::new(IP_B.into(), port)
}

fn a_addr(port: u16) -> SocketAddr {
    SocketAddr::new(IP_A.into(), port)
}

/// The TCP header of a frame, if it is a TCP segment.
fn tcp_of(frame: &[u8]) -> Option<(tcp::TcpHeader, usize)> {
    let f = ethernet::parse(frame).ok()?;
    let (h, p) = ipv4::parse(f.payload).ok()?;
    if h.protocol != crate::wire::proto::TCP {
        return None;
    }
    let (t, data) = tcp::parse(p).ok()?;
    Some((t, data.len()))
}

#[test]
fn handshake_small_transfer_and_close() {
    let mut s = Sim::new(1);
    let (c, l, a) = s.connect(80);
    assert_eq!(s.b.tcp_state(a), Ok(TcpState::Established));
    assert_eq!(s.a.peer_addr(c), Ok(b_addr(80)));
    assert_eq!(s.b.peer_addr(a).unwrap().ip(), IP_A);
    assert_eq!(s.b.local_addr(a), Ok(b_addr(80)));
    assert!(s.a.readiness(c).unwrap().contains(Readiness::WRITABLE));
    let got = s.transfer(true, c, a, b"hello, world", Duration::from_secs(10));
    assert_eq!(got, b"hello, world");
    let back = s.transfer(false, a, c, b"bye", Duration::from_secs(10));
    assert_eq!(back, b"bye");
    s.a.close(c).unwrap();
    s.b.close(a).unwrap();
    s.b.close(l).unwrap();
    // The active closer (A sent FIN first) sits in TIME-WAIT, then all is freed.
    s.run_for(Duration::from_secs(1));
    assert_eq!(s.b.socket_count(), 0, "passive closer is gone after LAST-ACK");
    assert_eq!(s.a.socket_count(), 1, "active closer waits in TIME-WAIT");
    s.run_for(Duration::from_secs(61));
    assert_eq!(s.a.socket_count(), 0);
}

#[test]
fn transfer_one_megabyte_clean() {
    let mut s = Sim::new(2);
    let (c, _l, a) = s.connect(5000);
    let data = pattern(1 << 20, 7);
    let got = s.transfer(true, c, a, &data, Duration::from_secs(60));
    assert!(got == data, "data differs");
    let info = s.a.tcp_info(c).unwrap();
    assert_eq!(info.stats.retransmits, 0);
    assert!(info.window_scales.is_some());
    assert!(info.timestamps);
    assert_eq!(info.send_mss, 1460 - 12);
}

#[test]
fn connect_to_closed_port_is_refused() {
    let mut s = Sim::new(3);
    let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    assert_eq!(s.a.connect(c, b_addr(9)), Err(Errno::EINPROGRESS));
    assert_eq!(s.a.connect(c, b_addr(9)), Err(Errno::EALREADY));
    assert!(s.run_until(Duration::from_secs(5), |s| s.a.readiness(c).unwrap().contains(Readiness::ERROR)));
    let r = s.a.readiness(c).unwrap();
    assert!(r.contains(Readiness::WRITABLE | Readiness::HANGUP));
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::ECONNREFUSED)));
    assert_eq!(s.a.take_error(c), Ok(None));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Closed));
    // The socket can try again; with a listener it now works.
    let l = s.b.socket(Domain::Inet, SockType::Stream, None).unwrap();
    s.b.bind(l, b_addr(9)).unwrap();
    s.b.listen(l, 1).unwrap();
    assert_eq!(s.a.connect(c, b_addr(9)), Err(Errno::EINPROGRESS));
    assert!(s.run_until(Duration::from_secs(5), |s| s.a.tcp_state(c) == Ok(TcpState::Established)));
}

#[test]
fn passive_close_and_half_close() {
    let mut s = Sim::new(4);
    let (c, _l, a) = s.connect(81);
    // B shuts down its sending side; A sees end of stream but can still send.
    s.b.shutdown(a, Shutdown::Write).unwrap();
    assert!(s.run_until(Duration::from_secs(2), |s| s.a.tcp_state(c) == Ok(TcpState::CloseWait)));
    let r = s.a.readiness(c).unwrap();
    assert!(r.contains(Readiness::READABLE | Readiness::READ_HANGUP | Readiness::WRITABLE));
    assert!(!r.contains(Readiness::HANGUP));
    let mut buf = [0u8; 8];
    assert_eq!(s.a.recv(c, &mut buf), Ok(0));
    assert_eq!(s.b.send(a, b"x"), Err(Errno::EPIPE));
    assert!(s.run_until(Duration::from_secs(1), |s| s.b.tcp_state(a) == Ok(TcpState::FinWait2)));
    let data = pattern(100_000, 1);
    let got = s.transfer(true, c, a, &data, Duration::from_secs(30));
    assert!(got == data);
    assert!(s.run_until(Duration::from_secs(2), |s| s.a.tcp_state(c) == Ok(TcpState::Closed)));
    assert_eq!(s.b.tcp_state(a), Ok(TcpState::TimeWait));
    assert!(s.a.readiness(c).unwrap().contains(Readiness::HANGUP));
}

#[test]
fn simultaneous_close() {
    let mut s = Sim::new(5);
    let (c, _l, a) = s.connect(82);
    // Both send FIN before seeing the other's.
    s.a.shutdown(c, Shutdown::Write).unwrap();
    s.b.shutdown(a, Shutdown::Write).unwrap();
    let mut seen_closing = false;
    assert!(s.run_until(Duration::from_secs(5), |s| {
        seen_closing |= s.a.tcp_state(c) == Ok(TcpState::Closing) || s.b.tcp_state(a) == Ok(TcpState::Closing);
        s.a.tcp_state(c) == Ok(TcpState::TimeWait) && s.b.tcp_state(a) == Ok(TcpState::TimeWait)
    }));
    assert!(seen_closing, "both sides pass through CLOSING");
    s.run_for(Duration::from_secs(61));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Closed));
    assert_eq!(s.b.tcp_state(a), Ok(TcpState::Closed));
}

#[test]
fn simultaneous_open() {
    let mut s = Sim::new(6);
    let ca = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    let cb = s.b.socket(Domain::Inet, SockType::Stream, None).unwrap();
    s.a.bind(ca, a_addr(7000)).unwrap();
    s.b.bind(cb, b_addr(7001)).unwrap();
    // Resolve ARP first so both SYNs cross on the wire.
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.send_to(u, b"x", Some(b_addr(9))).unwrap();
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.a.connect(ca, b_addr(7001)), Err(Errno::EINPROGRESS));
    assert_eq!(s.b.connect(cb, a_addr(7000)), Err(Errno::EINPROGRESS));
    let mut saw_syn_received = false;
    assert!(s.run_until(Duration::from_secs(10), |s| {
        saw_syn_received |= s.a.tcp_state(ca) == Ok(TcpState::SynReceived);
        s.a.tcp_state(ca) == Ok(TcpState::Established) && s.b.tcp_state(cb) == Ok(TcpState::Established)
    }));
    assert!(saw_syn_received);
    assert_eq!(s.a.tcp_info(ca).unwrap().stats.timeouts, 0, "no retransmission was needed");
    let got = s.transfer(true, ca, cb, b"crossed", Duration::from_secs(5));
    assert_eq!(got, b"crossed");
    let got = s.transfer(false, cb, ca, b"wires", Duration::from_secs(5));
    assert_eq!(got, b"wires");
}

#[test]
fn close_with_unread_data_resets_peer() {
    let mut s = Sim::new(7);
    let (c, _l, a) = s.connect(83);
    s.a.send(c, b"unread").unwrap();
    assert!(s.run_until(Duration::from_secs(2), |s| s.b.readiness(a).unwrap().contains(Readiness::READABLE)));
    s.b.close(a).unwrap();
    assert!(s.run_until(Duration::from_secs(2), |s| s.a.tcp_state(c) == Ok(TcpState::Closed)));
    assert!(s.a.readiness(c).unwrap().contains(Readiness::ERROR));
    let mut buf = [0u8; 4];
    assert_eq!(s.a.recv(c, &mut buf), Err(Errno::ECONNRESET));
    assert_eq!(s.a.send(c, b"x"), Err(Errno::EPIPE));
    assert_eq!(s.a.recv(c, &mut buf), Ok(0));
    assert_eq!(s.b.stats().tcp_rst_out, 1);
}

#[test]
fn data_after_close_resets() {
    let mut s = Sim::new(8);
    let (c, _l, a) = s.connect(84);
    s.b.close(a).unwrap();
    assert!(s.run_until(Duration::from_secs(2), |s| s.a.tcp_state(c) == Ok(TcpState::CloseWait)));
    // A writes into a connection whose other end has gone away.
    s.a.send(c, b"anyone there?").unwrap();
    assert!(s.run_until(Duration::from_secs(2), |s| s.a.tcp_state(c) == Ok(TcpState::Closed)));
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::ECONNRESET)));
}

/// Rebuild `template` (a TCP/IPv4 frame) with header `h` and no data.
pub(crate) fn forge_tcp(template: &[u8], h: &tcp::TcpHeader) -> Vec<u8> {
    let (ih, _) = ipv4::parse(&template[14..]).unwrap();
    let mut f = vec![0u8; 14 + 20 + h.header_len()];
    f[..14].copy_from_slice(&template[..14]);
    let mut ip = ih;
    ip.header_len = 20;
    ip.total_len = 20 + h.header_len();
    ipv4::emit(&mut f[14..], &ip);
    let ps = crate::wire::checksum::pseudo_v4(ih.src, ih.dst, crate::wire::proto::TCP, h.header_len());
    tcp::emit(&mut f[34..], h, 0, ps);
    f
}

/// Run until a TCP segment carrying `len` bytes goes `dir`; return it.
pub(crate) fn capture(s: &mut Sim, dir: Dir, len: usize) -> Vec<u8> {
    s.log = Some(Vec::new());
    let found = |s: &Sim| {
        s.log.as_ref().and_then(|l| {
            l.iter().find(|(d, f)| *d == dir && tcp_of(f).is_some_and(|(_, n)| n == len)).map(|(_, f)| f.clone())
        })
    };
    s.run_until(Duration::from_secs(2), |s| found(s).is_some());
    let f = found(s).expect("segment not seen");
    s.log = None;
    f
}

#[test]
fn rst_must_match_exactly() {
    let mut s = Sim::new(9);
    let (c, _l, a) = s.connect(85);
    s.b.send(a, b"pong").unwrap();
    let frame = capture(&mut s, Dir::BtoA, 4);
    s.run_for(Duration::from_millis(100));
    let mut buf = [0u8; 8];
    assert_eq!(s.a.recv(c, &mut buf), Ok(4));
    let (h, _) = tcp_of(&frame).unwrap();
    let rcv_nxt = h.seq.wrapping_add(4);
    let rst = |seq: u32| tcp::TcpHeader { seq, flags: tcp::RST, options: Default::default(), ..h };

    let acks = s.a.tcp_info(c).unwrap().stats.acks_out;
    s.da.inbox.push_back(forge_tcp(&frame, &rst(rcv_nxt.wrapping_add(100))));
    s.run_for(Duration::from_millis(50));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Established), "an in-window RST that is not exact");
    assert_eq!(s.a.tcp_info(c).unwrap().stats.acks_out, acks + 1, "gets a challenge ACK (RFC 5961)");

    s.da.inbox.push_back(forge_tcp(&frame, &rst(rcv_nxt.wrapping_sub(100_000))));
    s.run_for(Duration::from_millis(50));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Established), "an out-of-window RST is ignored");

    // A SYN on an established connection also only draws a challenge ACK.
    let syn = tcp::TcpHeader { seq: rcv_nxt, flags: tcp::SYN, options: Default::default(), ..h };
    let acks = s.a.tcp_info(c).unwrap().stats.acks_out;
    s.da.inbox.push_back(forge_tcp(&frame, &syn));
    s.run_for(Duration::from_millis(50));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Established));
    assert_eq!(s.a.tcp_info(c).unwrap().stats.acks_out, acks + 1);

    s.da.inbox.push_back(forge_tcp(&frame, &rst(rcv_nxt)));
    s.run_for(Duration::from_millis(50));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Closed), "the exact RST resets");
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::ECONNRESET)));
}

#[test]
fn segments_for_no_connection_get_rst() {
    let mut s = Sim::new(22);
    let (c, _l, a) = s.connect(96);
    s.b.send(a, b"pong").unwrap();
    let frame = capture(&mut s, Dir::BtoA, 4);
    let (h, _) = tcp_of(&frame).unwrap();
    s.run_for(Duration::from_millis(100));
    // An ACK to a port with nothing on it: RST with SEQ = SEG.ACK.
    let stray = tcp::TcpHeader { dst_port: 4444, flags: tcp::ACK, ack: 777, options: Default::default(), ..h };
    let before = s.a.stats().tcp_rst_out;
    s.da.inbox.push_back(forge_tcp(&frame, &stray));
    s.log = Some(Vec::new());
    s.run_for(Duration::from_millis(100));
    let log = s.log.take().unwrap();
    let (r, _) = log.iter().filter_map(|(_, f)| tcp_of(f)).find(|(t, _)| t.has(tcp::RST)).expect("RST");
    assert_eq!((r.seq, r.flags, r.src_port), (777, tcp::RST, 4444));
    assert_eq!(s.a.stats().tcp_rst_out, before + 1);
    // A RST to nothing is never answered.
    let stray = tcp::TcpHeader { dst_port: 4444, flags: tcp::RST, options: Default::default(), ..h };
    s.da.inbox.push_back(forge_tcp(&frame, &stray));
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.a.stats().tcp_rst_out, before + 1);
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Established), "the real connection is untouched");
}

#[test]
fn zero_window_probing_and_reopening() {
    let mut s = Sim::new(10);
    // Small receive buffer on B so the window closes quickly.
    let l = s.b.socket(Domain::Inet, SockType::Stream, None).unwrap();
    s.b.set_buffer_sizes(l, None, Some(4096)).unwrap();
    s.b.bind(l, b_addr(86)).unwrap();
    s.b.listen(l, 1).unwrap();
    let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    assert_eq!(s.a.connect(c, b_addr(86)), Err(Errno::EINPROGRESS));
    let mut a = None;
    assert!(s.run_until(Duration::from_secs(5), |s| {
        a = a.or(s.b.accept(l).ok().map(|x| x.0));
        a.is_some()
    }));
    let a = a.unwrap();
    let data = pattern(20_000, 3);
    let mut sent = 0;
    // Fill: B never reads.
    s.run_until(Duration::from_secs(1), |s| {
        if let Ok(n) = s.a.send(c, &data[sent..]) {
            sent += n;
        }
        false
    });
    assert_eq!(s.b.tcp_info(a).unwrap().recv_queued, 4096);
    // Let time pass: A probes the closed window with growing intervals.
    s.run_for(Duration::from_secs(30));
    let probes = s.a.tcp_info(c).unwrap().stats.window_probes;
    assert!((3..=10).contains(&probes), "probes: {probes}");
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Established), "a responsive peer keeps the connection");
    // Now read everything; the window reopens and the rest flows.
    let mut got = Vec::new();
    let mut buf = vec![0u8; 1024];
    let mut shut = false;
    assert!(s.run_until(Duration::from_secs(120), |s| {
        while sent < data.len() {
            match s.a.send(c, &data[sent..]) {
                Ok(n) => sent += n,
                Err(_) => break,
            }
        }
        if sent == data.len() && !shut {
            s.a.shutdown(c, Shutdown::Write).unwrap();
            shut = true;
        }
        loop {
            match s.b.recv(a, &mut buf) {
                Ok(0) => return true,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(_) => return false,
            }
        }
    }));
    assert!(got == data);
}

#[test]
fn zero_window_with_lost_window_update() {
    let mut s = Sim::new(11);
    let l = s.b.socket(Domain::Inet, SockType::Stream, None).unwrap();
    s.b.set_buffer_sizes(l, None, Some(2048)).unwrap();
    s.b.bind(l, b_addr(87)).unwrap();
    s.b.listen(l, 1).unwrap();
    let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    assert_eq!(s.a.connect(c, b_addr(87)), Err(Errno::EINPROGRESS));
    let mut a = None;
    s.run_until(Duration::from_secs(5), |s| {
        a = a.or(s.b.accept(l).ok().map(|x| x.0));
        a.is_some()
    });
    let a = a.unwrap();
    let data = pattern(8000, 4);
    let mut sent = 0;
    s.run_until(Duration::from_secs(1), |s| {
        if let Ok(n) = s.a.send(c, &data[sent..]) {
            sent += n;
        }
        false
    });
    // Drop every B->A segment for a while: the window update is lost, and
    // only the persist timer gets things going again.
    s.filter = Some(Box::new(|d, _| d == Dir::AtoB));
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = s.b.recv(a, &mut buf) {
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    s.run_for(Duration::from_millis(500));
    s.filter = None;
    let mut shut = false;
    assert!(s.run_until(Duration::from_secs(120), |s| {
        while sent < data.len() {
            match s.a.send(c, &data[sent..]) {
                Ok(n) => sent += n,
                Err(_) => break,
            }
        }
        if sent == data.len() && !shut {
            s.a.shutdown(c, Shutdown::Write).unwrap();
            shut = true;
        }
        loop {
            match s.b.recv(a, &mut buf) {
                Ok(0) => return true,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(_) => return false,
            }
        }
    }));
    assert!(got == data);
}

#[test]
fn listen_backlog_limits_and_recovers() {
    let mut s = Sim::new(12);
    let l = s.b.socket(Domain::Inet, SockType::Stream, None).unwrap();
    s.b.bind(l, b_addr(88)).unwrap();
    s.b.listen(l, 2).unwrap();
    let clients: Vec<_> = (0..4)
        .map(|_| {
            let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
            assert_eq!(s.a.connect(c, b_addr(88)), Err(Errno::EINPROGRESS));
            c
        })
        .collect();
    s.run_for(Duration::from_millis(500));
    let established = clients.iter().filter(|&&c| s.a.tcp_state(c) == Ok(TcpState::Established)).count();
    assert_eq!(established, 2, "only the backlog's worth complete");
    assert!(s.b.stats().syn_dropped >= 2);
    assert!(s.b.readiness(l).unwrap().contains(Readiness::READABLE));
    let mut accepted = vec![s.b.accept(l).unwrap().0, s.b.accept(l).unwrap().0];
    assert_eq!(s.b.accept(l), Err(Errno::EAGAIN));
    // The others get in on a SYN retransmission.
    assert!(s.run_until(Duration::from_secs(10), |s| {
        if let Ok((h, _)) = s.b.accept(l) {
            accepted.push(h);
        }
        accepted.len() == 4
    }));
    for &c in &clients {
        assert_eq!(s.a.tcp_state(c), Ok(TcpState::Established));
    }
    // Closing the listener resets connections nobody accepted.
    let c5 = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    assert_eq!(s.a.connect(c5, b_addr(88)), Err(Errno::EINPROGRESS));
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.a.tcp_state(c5), Ok(TcpState::Established));
    s.b.close(l).unwrap();
    s.run_for(Duration::from_millis(100));
    assert_eq!(s.a.take_error(c5), Ok(Some(Errno::ECONNRESET)));
    for &h in &accepted {
        assert_eq!(s.b.tcp_state(h), Ok(TcpState::Established), "accepted ones live on");
    }
}

#[test]
fn nagle_coalesces_small_writes_and_nodelay_does_not() {
    for nodelay in [false, true] {
        let mut s = Sim::new(13);
        s.link.delay = Duration::from_millis(50);
        let (c, _l, a) = s.connect(89);
        s.a.set_nodelay(c, nodelay).unwrap();
        let before = s.a.tcp_info(c).unwrap().stats.segs_out;
        // 20 one-byte writes, 10 ms apart, on a 100 ms round trip.
        for _ in 0..20 {
            s.a.send(c, b"x").unwrap();
            s.run_for(Duration::from_millis(10));
        }
        s.run_for(Duration::from_millis(500));
        let segs = s.a.tcp_info(c).unwrap().stats.segs_out - before;
        let mut buf = [0u8; 64];
        assert_eq!(s.b.recv(a, &mut buf), Ok(20));
        if nodelay {
            assert!(segs >= 20, "nodelay: {segs} segments");
        } else {
            assert!(segs <= 6, "Nagle: {segs} segments");
        }
    }
}

#[test]
fn delayed_ack_acks_every_second_segment() {
    let mut s = Sim::new(14);
    let (c, _l, a) = s.connect(90);
    let data = pattern(200 * 1448, 5);
    let before = s.b.tcp_info(a).unwrap().stats.acks_out;
    let got = s.transfer(true, c, a, &data, Duration::from_secs(10));
    assert!(got == data);
    let acks = s.b.tcp_info(a).unwrap().stats.acks_out - before;
    assert!((90..=130).contains(&acks), "{acks} ACKs for 200 segments");
    // A lone segment is acknowledged after the delayed-ACK timeout.
    let before = s.a.tcp_info(c).unwrap().stats.acks_out;
    s.b.send(a, b"one").unwrap();
    s.run_for(Duration::from_millis(20));
    assert_eq!(s.a.tcp_info(c).unwrap().stats.acks_out, before, "not yet");
    s.run_for(Duration::from_millis(40));
    assert_eq!(s.a.tcp_info(c).unwrap().stats.acks_out, before + 1);
}

#[test]
fn retransmission_gives_up_with_etimedout() {
    let mut s = Sim::new(15);
    let (c, _l, _a) = s.connect(91);
    // Segments vanish; ARP still works.
    s.filter = Some(Box::new(|_, f| tcp_of(f).is_none()));
    let start = s.now;
    s.a.send(c, b"lost forever").unwrap();
    assert!(s.run_until(Duration::from_secs(3600), |s| s.a.tcp_state(c) == Ok(TcpState::Closed)));
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::ETIMEDOUT)));
    let info = s.a.tcp_info(c).unwrap();
    assert_eq!(info.stats.timeouts, 15);
    // 16 timeouts from 200 ms doubling to the 60 s cap: 0.2 + ... + 51.2 + 7 * 60.
    let took = s.now.since(start).as_millis();
    assert!((522_000..523_000).contains(&took), "gave up after {took} ms");
}

#[test]
fn dead_neighbor_fails_with_ehostunreach() {
    let mut s = Sim::new(23);
    let (c, _l, _a) = s.connect(97);
    s.cut = true;
    s.a.send(c, b"anyone?").unwrap();
    assert!(s.run_until(Duration::from_secs(3600), |s| s.a.tcp_state(c) == Ok(TcpState::Closed)));
    // The ARP entry ran out and could not be renewed: that is what is
    // reported, not a bare timeout.
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::EHOSTUNREACH)));
}

#[test]
fn syn_retries_then_etimedout() {
    let mut s = Sim::new(16);
    let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    // The SYNs vanish; ARP still works, so this is a plain timeout.
    s.filter = Some(Box::new(|d, f| !(d == Dir::AtoB && tcp_of(f).is_some())));
    assert_eq!(s.a.connect(c, b_addr(92)), Err(Errno::EINPROGRESS));
    let start = s.now;
    assert!(s.run_until(Duration::from_secs(600), |s| s.a.tcp_state(c) == Ok(TcpState::Closed)));
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::ETIMEDOUT)));
    // 1 + 2 + 4 + 8 + 16 + 32 + 60 (capped) seconds.
    let took = s.now.since(start).as_secs();
    assert!((120..=130).contains(&took), "took {took} s");
}

#[test]
fn keepalive_detects_dead_peer() {
    let mut s = Sim::new(17);
    let (c, _l, _a) = s.connect(93);
    s.a.set_keepalive(c, true).unwrap();
    // A healthy idle connection survives with keepalives answered.
    s.run_for(Duration::from_secs(3 * 3600));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Established));
    assert!(s.a.tcp_info(c).unwrap().stats.keepalive_probes >= 1);
    // Peer vanishes.
    s.cut = true;
    s.run_for(Duration::from_secs(3 * 3600));
    assert_eq!(s.a.tcp_state(c), Ok(TcpState::Closed));
    assert_eq!(s.a.take_error(c), Ok(Some(Errno::ETIMEDOUT)));
}

#[test]
fn bind_and_port_rules() {
    let mut s = Sim::new(18);
    let t1 = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    let t2 = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    s.a.bind(t1, a_addr(1234)).unwrap();
    assert_eq!(s.a.bind(t1, a_addr(1235)), Err(Errno::EINVAL));
    assert_eq!(s.a.bind(t2, a_addr(1234)), Err(Errno::EADDRINUSE));
    assert_eq!(s.a.bind(t2, SocketAddr::new(core::net::Ipv4Addr::UNSPECIFIED.into(), 1234)), Err(Errno::EADDRINUSE));
    assert_eq!(s.a.bind(t2, b_addr(1)), Err(Errno::EADDRNOTAVAIL));
    assert_eq!(s.a.bind(t2, "[::1]:5".parse().unwrap()), Err(Errno::EAFNOSUPPORT));
    // UDP has its own port space.
    let u = s.a.socket(Domain::Inet, SockType::Datagram, None).unwrap();
    s.a.bind(u, a_addr(1234)).unwrap();
    // Ephemeral ports are in range and distinct.
    let mut ports = Vec::new();
    for _ in 0..50 {
        let t = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
        s.a.bind(t, a_addr(0)).unwrap();
        let p = s.a.local_addr(t).unwrap().port();
        assert!((32768..=60999).contains(&p));
        assert!(!ports.contains(&p));
        ports.push(p);
    }
    // Listen auto-binds; stale handles fail.
    let t3 = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    s.a.listen(t3, 5).unwrap();
    assert_ne!(s.a.local_addr(t3).unwrap().port(), 0);
    s.a.close(t3).unwrap();
    assert_eq!(s.a.listen(t3, 5), Err(Errno::EBADF));
    let t4 = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    assert_eq!(t4.index(), t3.index(), "the slot is reused");
    assert_eq!(s.a.readiness(t3), Err(Errno::EBADF), "with a new generation");
    // Wrong operations.
    assert_eq!(s.a.listen(u, 1), Err(Errno::EOPNOTSUPP));
    assert_eq!(s.a.accept(t1), Err(Errno::EINVAL));
    let mut buf = [0u8; 4];
    assert_eq!(s.a.recv(t1, &mut buf), Err(Errno::ENOTCONN));
    assert_eq!(s.a.send(t1, b"x"), Err(Errno::ENOTCONN));
    assert_eq!(s.a.shutdown(t1, Shutdown::Both), Err(Errno::ENOTCONN));
    assert_eq!(s.a.peer_addr(t1), Err(Errno::ENOTCONN));
    assert_eq!(s.a.socket(Domain::Inet, SockType::Stream, Some(crate::Protocol::Udp)), Err(Errno::EPROTONOSUPPORT));
}

#[test]
fn time_wait_holds_the_port_until_reuse_addr() {
    let mut s = Sim::new(19);
    let (c, l, a) = s.connect(94);
    // Server closes first: the server side ends up in TIME-WAIT.
    s.b.close(a).unwrap();
    s.run_for(Duration::from_millis(100));
    s.a.close(c).unwrap();
    s.run_for(Duration::from_millis(100));
    s.b.close(l).unwrap();
    assert_eq!(s.b.socket_count(), 1, "the TIME-WAIT connection");
    let l2 = s.b.socket(Domain::Inet, SockType::Stream, None).unwrap();
    assert_eq!(s.b.bind(l2, b_addr(94)), Err(Errno::EADDRINUSE));
    s.b.set_reuse_addr(l2, true).unwrap();
    s.b.bind(l2, b_addr(94)).unwrap();
    s.b.listen(l2, 1).unwrap();
}

#[test]
fn time_wait_lasts_two_msl() {
    let mut s = Sim::new(20);
    let (c, _l, a) = s.connect(95);
    s.a.close(c).unwrap();
    s.run_for(Duration::from_millis(100));
    s.b.close(a).unwrap();
    s.run_for(Duration::from_millis(100));
    let entered = s.now;
    assert_eq!(s.a.socket_count(), 1);
    s.run_until(Duration::from_secs(120), |s| s.a.socket_count() == 0);
    let held = s.now.since(entered).as_secs();
    assert!((59..=61).contains(&held), "TIME-WAIT held {held} s");
}

#[test]
fn loopback_connection_and_own_address() {
    let mut s = Sim::new(21);
    for target in [core::net::Ipv4Addr::LOCALHOST, IP_A] {
        let l = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
        s.a.bind(l, SocketAddr::new(core::net::Ipv4Addr::UNSPECIFIED.into(), 7777)).unwrap();
        s.a.listen(l, 1).unwrap();
        let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
        assert_eq!(s.a.connect(c, SocketAddr::new(target.into(), 7777)), Err(Errno::EINPROGRESS));
        let mut acc = None;
        assert!(s.run_until(Duration::from_secs(2), |s| {
            acc = acc.or(s.a.accept(l).ok());
            acc.is_some()
        }));
        let (srv, peer) = acc.unwrap();
        assert_eq!(peer.ip(), target);
        let data = pattern(300_000, 9);
        let got = s.transfer_local(c, srv, &data);
        assert!(got == data);
        s.a.close(c).unwrap();
        s.a.close(srv).unwrap();
        s.a.close(l).unwrap();
        s.run_for(Duration::from_secs(70));
        assert_eq!(s.a.socket_count(), 0);
    }
    assert_eq!(s.frames[0], 0, "loopback traffic never reaches the device");
}

#[test]
fn small_sends_to_a_local_peer_arrive_within_the_same_instant() {
    // A peer on this host is acknowledged at once, so Nagle does not hold
    // the next small segment back: polling while loopback packets wait
    // (what the kernel's interface does) leaves every byte in the
    // receiver's queue without the clock moving.
    let mut s = Sim::new(22);
    let l = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    s.a.bind(l, SocketAddr::new(core::net::Ipv4Addr::UNSPECIFIED.into(), 7778)).unwrap();
    s.a.listen(l, 1).unwrap();
    let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    let _ = s.a.connect(c, SocketAddr::new(core::net::Ipv4Addr::LOCALHOST.into(), 7778));
    let mut acc = None;
    assert!(s.run_until(Duration::from_secs(2), |s| {
        acc = acc.or(s.a.accept(l).ok());
        acc.is_some()
    }));
    let (srv, _) = acc.unwrap();
    let now = s.now;
    for part in [&b"ab"[..], b"cd", b"ef"] {
        assert_eq!(s.a.send_to(c, part, None), Ok(2));
        s.a.poll(&mut s.da, now);
        while s.a.loopback_pending() {
            s.a.poll(&mut s.da, now);
        }
    }
    let mut buf = [0u8; 16];
    let got = s.a.recv_from(srv, &mut buf, crate::RecvFlags { peek: false }).unwrap();
    assert_eq!(&buf[..got.len], b"abcdef");
}
