//! TCP over a bad link: loss, reordering, duplication, corruption. Every
//! transfer must arrive byte for byte.

use super::pattern;
use crate::sim::{Dir, IP_A, IP_B, LinkConfig, Sim, config};
use crate::wire::{ethernet, ipv4, tcp};
use crate::{Domain, Errno, SockType, TcpState};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;

fn bad_link(loss: u32, dup: u32, corrupt: u32, jitter_ms: u64) -> LinkConfig {
    LinkConfig {
        loss,
        duplicate: dup,
        corrupt,
        delay: Duration::from_millis(10),
        jitter: Duration::from_millis(jitter_ms),
    }
}

/// Send as much of `data` as the socket takes; shut down writing at the end.
fn pump(st: &mut crate::Stack, h: crate::SocketHandle, data: &[u8], sent: &mut usize, shut: &mut bool) {
    while *sent < data.len() {
        match st.send(h, &data[*sent..]) {
            Ok(n) => *sent += n,
            Err(Errno::EAGAIN) => break,
            Err(e) => panic!("send: {e:?}"),
        }
    }
    if *sent == data.len() && !*shut {
        st.shutdown(h, crate::Shutdown::Write).unwrap();
        *shut = true;
    }
}

/// Read everything available; note end of stream.
fn drain(st: &mut crate::Stack, h: crate::SocketHandle, got: &mut Vec<u8>, eof: &mut bool, buf: &mut [u8]) {
    while !*eof {
        match st.recv(h, buf) {
            Ok(0) => *eof = true,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(Errno::EAGAIN) => break,
            Err(e) => panic!("recv: {e:?}"),
        }
    }
}

/// Transfer `size` bytes each way at once over `link` and check them.
/// Returns the simulation and A's socket.
fn both_ways(seed: u64, link: LinkConfig, size: usize) -> (Sim, crate::SocketHandle) {
    let mut s = Sim::new(seed);
    s.link = link;
    let (c, _l, a) = s.connect(6000);
    let up = pattern(size, seed);
    let down = pattern(size, seed ^ 0xffff);
    let (mut sent, mut shut, mut eof) = ([0usize; 2], [false; 2], [false; 2]);
    let mut got: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let mut buf = alloc::vec![0u8; 32 * 1024];
    let done = s.run_until(Duration::from_secs(3600), |s| {
        pump(&mut s.a, c, &up, &mut sent[0], &mut shut[0]);
        pump(&mut s.b, a, &down, &mut sent[1], &mut shut[1]);
        drain(&mut s.b, a, &mut got[0], &mut eof[0], &mut buf);
        drain(&mut s.a, c, &mut got[1], &mut eof[1], &mut buf);
        eof[0] && eof[1]
    });
    assert!(done, "seed {seed}: up {}/{} down {}/{}", got[0].len(), size, got[1].len(), size);
    assert!(got[0] == up, "seed {seed}: upstream data differs");
    assert!(got[1] == down, "seed {seed}: downstream data differs");
    (s, c)
}

#[test]
fn megabytes_through_random_loss() {
    // 2% loss each way, several seeds, 2 MiB in each direction at once.
    for seed in 1..=3 {
        let (s, _) = both_ways(seed, bad_link(20, 0, 0, 0), 2 << 20);
        assert!(s.a.stats().tx_frames > 2000);
    }
}

#[test]
fn heavy_loss_still_delivers() {
    // 10% loss each way.
    let (s, c) = both_ways(11, bad_link(100, 0, 0, 0), 256 * 1024);
    assert!(s.a.tcp_info(c).unwrap().stats.timeouts > 0);
}

#[test]
fn reordering_duplication_and_corruption() {
    for seed in 20..=22 {
        let (s, _) = both_ways(seed, bad_link(10, 20, 10, 15), 1 << 20);
        assert!(s.a.stats().rx_checksum + s.b.stats().rx_checksum > 0, "corruption was caught by checksums");
    }
}

#[test]
fn four_megabytes_with_everything_at_once() {
    let mut s = Sim::new(31);
    s.link = bad_link(15, 10, 5, 8);
    let (c, _l, a) = s.connect(6001);
    let data = pattern(4 << 20, 31);
    let got = s.transfer(true, c, a, &data, Duration::from_secs(3600));
    assert!(got == data);
    let info = s.a.tcp_info(c).unwrap();
    assert!(info.stats.retransmits > 0);
    assert!(info.stats.fast_retransmits > 0, "losses are repaired without a timeout too: {:?}", info.stats);
    assert!(info.stats.acks_out < 50, "the sender does not answer reordered ACKs: {:?}", info.stats);
}

#[test]
fn single_loss_is_repaired_by_fast_retransmit() {
    let mut s = Sim::new(32);
    let (c, _l, a) = s.connect(6002);
    // Drop exactly one data segment, the 30th.
    let mut n = 0;
    s.filter = Some(Box::new(move |d, f| {
        if d == Dir::AtoB
            && let Ok(e) = ethernet::parse(f)
            && let Ok((ih, p)) = ipv4::parse(e.payload)
            && ih.protocol == crate::wire::proto::TCP
            && let Ok((_, data)) = tcp::parse(p)
            && !data.is_empty()
        {
            n += 1;
            return n != 30;
        }
        true
    }));
    let data = pattern(400 * 1448, 2);
    let got = s.transfer(true, c, a, &data, Duration::from_secs(60));
    assert!(got == data);
    let st = s.a.tcp_info(c).unwrap().stats;
    assert_eq!(st.fast_retransmits, 1, "{st:?}");
    assert_eq!(st.timeouts, 0, "{st:?}");
    assert_eq!(st.retransmits, 1, "{st:?}");
}

#[test]
fn window_scaling_fills_a_long_fat_pipe() {
    // 100 ms each way: a 64 KiB window alone would cap the rate at
    // 320 KiB/s. With 1 MiB buffers and window scaling it goes far faster.
    let mut ca = config(IP_A, 1);
    let mut cb = config(IP_B, 2);
    ca.tcp.send_buffer = 1 << 20;
    cb.tcp.recv_buffer = 1 << 20;
    let mut s = Sim::with_configs(33, ca, cb, 1500);
    s.link.delay = Duration::from_millis(100);
    let (c, _l, a) = s.connect(6003);
    let ws = s.a.tcp_info(c).unwrap().window_scales.expect("negotiated");
    assert_eq!(ws.0, 5, "B scales its 1 MiB window by 2^5");
    let start = s.now;
    let data = pattern(8 << 20, 3);
    let got = s.transfer(true, c, a, &data, Duration::from_secs(600));
    assert!(got == data);
    let secs = s.now.since(start).as_secs_f64();
    let rate = data.len() as f64 / secs;
    // Slow start and delayed ACKs take a while to open a 1 MiB window at
    // a 200 ms round trip; still several times the unscaled limit.
    assert!(rate > 1.5 * 1024.0 * 1024.0, "{rate} B/s over {secs} s");
}

#[test]
fn no_window_scaling_when_the_peer_does_not_offer_it() {
    let mut cb = config(IP_B, 2);
    cb.tcp.window_scaling = false;
    cb.tcp.timestamps = false;
    let mut s = Sim::with_configs(34, config(IP_A, 1), cb, 1500);
    let (c, _l, a) = s.connect(6004);
    let info = s.a.tcp_info(c).unwrap();
    assert_eq!(info.window_scales, None);
    assert!(!info.timestamps);
    assert_eq!(info.send_mss, 1460);
    let data = pattern(1 << 20, 4);
    assert!(s.transfer(true, c, a, &data, Duration::from_secs(60)) == data);
}

#[test]
fn mss_follows_the_smaller_mtu() {
    // B's interface has an MTU of 576; A must send segments that fit.
    let mut s = Sim::with_configs(35, config(IP_A, 1), config(IP_B, 2), 1500);
    s.db.mtu = 576;
    s.b = crate::Stack::for_device(&s.db, config(IP_B, 2));
    let (c, _l, a) = s.connect(6005);
    assert_eq!(s.a.tcp_info(c).unwrap().send_mss, 536 - 12);
    let mut biggest = 0;
    s.filter = Some(Box::new(move |_, f| {
        biggest = biggest.max(f.len());
        assert!(biggest <= 576 + 14, "frame of {biggest} bytes");
        true
    }));
    let data = pattern(200_000, 5);
    assert!(s.transfer(true, c, a, &data, Duration::from_secs(60)) == data);
    let back = pattern(200_000, 6);
    assert!(s.transfer(false, a, c, &back, Duration::from_secs(60)) == back);
}

#[test]
fn sequence_numbers_wrap_around() {
    let mut s = Sim::new(36);
    // Start both sides 100 KB below 2^32 so the transfer crosses zero.
    s.a.isn_override = Some(u32::MAX - 100_000);
    s.b.isn_override = Some(u32::MAX - 50_000);
    s.link = bad_link(20, 5, 0, 5);
    let (c, _l, a) = s.connect(6006);
    let data = pattern(1 << 20, 7);
    assert!(s.transfer(true, c, a, &data, Duration::from_secs(600)) == data);
    let back = pattern(300_000, 8);
    assert!(s.transfer(false, a, c, &back, Duration::from_secs(600)) == back);
    assert!(s.run_until(Duration::from_secs(5), |s| s.a.tcp_state(c) == Ok(TcpState::TimeWait)));
}

#[test]
fn many_connections_in_parallel() {
    let mut s = Sim::new(37);
    s.link = bad_link(10, 0, 0, 5);
    let l = s.b.socket(Domain::Inet, SockType::Stream, None).unwrap();
    s.b.bind(l, SocketAddr::new(IP_B.into(), 7000)).unwrap();
    s.b.listen(l, 64).unwrap();
    let n = 16;
    let clients: Vec<_> = (0..n)
        .map(|_| {
            let c = s.a.socket(Domain::Inet, SockType::Stream, None).unwrap();
            assert_eq!(s.a.connect(c, SocketAddr::new(IP_B.into(), 7000)), Err(Errno::EINPROGRESS));
            c
        })
        .collect();
    let mut servers = Vec::new();
    assert!(s.run_until(Duration::from_secs(60), |s| {
        while let Ok((h, peer)) = s.b.accept(l) {
            servers.push((h, peer));
        }
        servers.len() == n
    }));
    // Each client sends its own pattern; match them up by port.
    let datas: Vec<Vec<u8>> = (0..n).map(|i| pattern(64 * 1024, 100 + i as u64)).collect();
    let mut sent = alloc::vec![0usize; n];
    let mut got: Vec<Vec<u8>> = alloc::vec![Vec::new(); n];
    let mut buf = alloc::vec![0u8; 8192];
    let ports: Vec<u16> = clients.iter().map(|&c| s.a.local_addr(c).unwrap().port()).collect();
    assert!(s.run_until(Duration::from_secs(600), |s| {
        for i in 0..n {
            if let Ok(k) = s.a.send(clients[i], &datas[i][sent[i]..]) {
                sent[i] += k;
            }
        }
        for &(h, peer) in &servers {
            let i = ports.iter().position(|&p| p == peer.port()).unwrap();
            while let Ok(k) = s.b.recv(h, &mut buf) {
                if k == 0 {
                    break;
                }
                got[i].extend_from_slice(&buf[..k]);
            }
        }
        (0..n).all(|i| got[i].len() == datas[i].len())
    }));
    for i in 0..n {
        assert!(got[i] == datas[i], "connection {i}");
    }
}
