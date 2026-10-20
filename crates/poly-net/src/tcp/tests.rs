//! Unit tests of the TCP state machine with hand-made segments: one block
//! talks to a scripted peer.

use super::*;

const ISS: u32 = 100;
const PEER_ISS: u32 = 5000;

fn cfg() -> TcpConfig {
    TcpConfig { timestamps: false, window_scaling: false, ..TcpConfig::default() }
}

fn local() -> SocketAddr {
    "10.0.0.1:1000".parse().unwrap()
}

fn remote() -> SocketAddr {
    "10.0.0.2:80".parse().unwrap()
}

fn t(ms: u64) -> Instant {
    Instant::from_millis(ms)
}

fn seg(seq: u32, ack: u32, flags: u8) -> TcpHeader {
    TcpHeader { src_port: 80, dst_port: 1000, seq, ack, flags, window: 10_000, ..Default::default() }
}

fn out(c: &mut Tcb, now: Instant) -> Option<(TcpHeader, alloc::vec::Vec<u8>)> {
    let mut buf = [0u8; 2000];
    c.dispatch(now, &mut buf).map(|(h, n)| (h, buf[..n].to_vec()))
}

/// A connection after a three-way handshake with a peer whose MSS is 1000.
fn established(cfg: TcpConfig) -> Tcb {
    let mut c = Tcb::new(cfg, local(), t(0));
    c.connect(local(), remote(), Seq(ISS), 1460, t(0));
    let (syn, _) = out(&mut c, t(0)).unwrap();
    assert_eq!((syn.flags, syn.seq), (SYN, ISS));
    assert_eq!(syn.options.mss, Some(1460));
    let mut sa = seg(PEER_ISS, ISS + 1, SYN | ACK);
    sa.options.mss = Some(1000);
    assert!(c.input(t(10), &sa, &[]).is_none());
    assert_eq!(c.state, TcpState::Established);
    let (ack, _) = out(&mut c, t(10)).unwrap();
    assert_eq!((ack.flags, ack.seq, ack.ack), (ACK, ISS + 1, PEER_ISS + 1));
    assert!(out(&mut c, t(10)).is_none());
    c
}

#[test]
fn handshake_takes_the_peer_mss_and_measures_rtt() {
    let c = established(cfg());
    assert_eq!(c.send_mss(), 1000);
    assert_eq!(c.srtt(), Some(Duration::from_millis(10)));
}

#[test]
fn ack_for_unsent_data_draws_an_ack_and_is_dropped() {
    let mut c = established(cfg());
    let mut h = seg(PEER_ISS + 1, ISS + 1 + 500, ACK);
    let r = c.input(t(20), &h, b"data");
    assert!(r.is_none());
    let (a, _) = out(&mut c, t(20)).unwrap();
    assert_eq!(a.ack, PEER_ISS + 1, "the data in that segment was not taken");
    h.ack = ISS + 1;
    c.input(t(20), &h, b"data");
    assert_eq!(c.recv_queued(), 4);
}

#[test]
fn out_of_order_fin_waits_for_the_gap() {
    let mut c = established(cfg());
    let base = PEER_ISS + 1;
    c.input(t(20), &seg(base + 10, ISS + 1, ACK | FIN), &[2; 10]);
    assert_eq!(c.state, TcpState::Established);
    let (dup, _) = out(&mut c, t(20)).unwrap();
    assert_eq!(dup.ack, base, "duplicate ACK for the gap");
    c.input(t(21), &seg(base, ISS + 1, ACK), &[1; 10]);
    assert_eq!(c.state, TcpState::CloseWait);
    let (a, _) = out(&mut c, t(21)).unwrap();
    assert_eq!(a.ack, base + 21, "data and FIN acknowledged");
    let mut buf = [0u8; 32];
    assert_eq!(c.recv(&mut buf, false), Ok(20));
    assert_eq!(&buf[..20], &[[1u8; 10], [2u8; 10]].concat()[..]);
    assert_eq!(c.recv(&mut buf, false), Ok(0));
}

#[test]
fn data_beyond_the_window_is_cut() {
    let mut c = established(TcpConfig { recv_buffer: 100, ..cfg() });
    let base = PEER_ISS + 1;
    c.input(t(20), &seg(base, ISS + 1, ACK | FIN), &[7; 150]);
    assert_eq!(c.recv_queued(), 100);
    assert_eq!(c.state, TcpState::Established, "the FIN was past the window");
    let (a, _) = out(&mut c, t(20)).unwrap();
    assert_eq!((a.ack, a.window), (base + 100, 0));
    // A zero-window probe is answered but not taken.
    c.input(t(30), &seg(base + 100, ISS + 1, ACK), &[8]);
    let (a, _) = out(&mut c, t(30)).unwrap();
    assert_eq!((a.ack, a.window), (base + 100, 0));
    // Reading opens the window and announces it.
    let mut buf = [0u8; 100];
    assert_eq!(c.recv(&mut buf, false), Ok(100));
    let (a, _) = out(&mut c, t(40)).unwrap();
    assert_eq!(a.window, 100);
}

#[test]
fn syn_sent_ack_and_rst_checks() {
    let mut c = Tcb::new(cfg(), local(), t(0));
    c.connect(local(), remote(), Seq(ISS), 1460, t(0));
    out(&mut c, t(0)).unwrap();
    // A SYN-ACK acknowledging something else gets a RST with SEQ = SEG.ACK.
    let rst = c.input(t(5), &seg(PEER_ISS, 999, SYN | ACK), &[]).expect("RST");
    assert_eq!((rst.flags, rst.seq, rst.dst_port), (RST, 999, 80));
    assert_eq!(c.state, TcpState::SynSent);
    // A RST without ACK, or with a wrong one, is ignored.
    assert!(c.input(t(6), &seg(PEER_ISS, 0, RST), &[]).is_none());
    assert!(c.input(t(6), &seg(PEER_ISS, 7, RST | ACK), &[]).is_none());
    assert_eq!(c.state, TcpState::SynSent);
    // A RST acknowledging our SYN refuses the connection.
    c.input(t(7), &seg(0, ISS + 1, RST | ACK), &[]);
    assert_eq!(c.state, TcpState::Closed);
    assert_eq!(c.error, Some(Errno::ECONNREFUSED));
}

#[test]
fn syn_is_retransmitted_with_backoff() {
    let mut c = Tcb::new(cfg(), local(), t(0));
    c.connect(local(), remote(), Seq(ISS), 1460, t(0));
    out(&mut c, t(0)).unwrap();
    assert_eq!(c.timer_deadline(), Some(t(1000)));
    c.on_timers(t(1000));
    let (syn, _) = out(&mut c, t(1000)).unwrap();
    assert_eq!((syn.flags, syn.seq), (SYN, ISS));
    assert_eq!(c.timer_deadline(), Some(t(3000)));
    // The answer to the retransmitted SYN gives no RTT sample (Karn), and
    // data starts with an RTO of at least 3 s (RFC 6298 5.7).
    c.input(t(1010), &seg(PEER_ISS, ISS + 1, SYN | ACK), &[]);
    assert_eq!(c.srtt(), None);
    assert!(c.rtt.rto() >= Duration::from_secs(3));
}

#[test]
fn nagle_holds_small_segments_until_acked() {
    let mut c = established(cfg());
    c.send(b"a").unwrap();
    let (s1, d1) = out(&mut c, t(20)).unwrap();
    assert_eq!(d1, b"a");
    c.send(b"b").unwrap();
    c.send(b"c").unwrap();
    assert!(out(&mut c, t(21)).is_none(), "held while 'a' is unacknowledged");
    c.input(t(30), &seg(PEER_ISS + 1, s1.seq + 1, ACK), &[]);
    let (_, d2) = out(&mut c, t(30)).unwrap();
    assert_eq!(d2, b"bc");
    c.nodelay = true;
    c.send(b"d").unwrap();
    assert_eq!(out(&mut c, t(31)).unwrap().1, b"d");
}

#[test]
fn orphan_in_fin_wait_2_times_out() {
    let mut c = established(cfg());
    assert!(!c.close(t(20)));
    let (fin, _) = out(&mut c, t(20)).unwrap();
    assert_eq!(fin.flags, ACK | FIN);
    assert_eq!(c.state, TcpState::FinWait1);
    c.input(t(30), &seg(PEER_ISS + 1, fin.seq + 1, ACK), &[]);
    assert_eq!(c.state, TcpState::FinWait2);
    c.on_timers(t(30 + 59_999));
    assert_eq!(c.state, TcpState::FinWait2);
    c.on_timers(t(30 + 60_000));
    assert_eq!(c.state, TcpState::Closed);
    assert_eq!(c.error, None);
}

#[test]
fn paws_drops_old_segments() {
    let tcfg = TcpConfig { timestamps: true, ..cfg() };
    let mut c = Tcb::new(tcfg, local(), t(0));
    c.connect(local(), remote(), Seq(ISS), 1460, t(0));
    out(&mut c, t(0)).unwrap();
    let mut sa = seg(PEER_ISS, ISS + 1, SYN | ACK);
    sa.options.timestamps = Some((1000, 0));
    c.input(t(10), &sa, &[]);
    assert!(c.timestamps_enabled());
    let (ack, _) = out(&mut c, t(10)).unwrap();
    assert_eq!(ack.options.timestamps.map(|t| t.1), Some(1000), "echoes the peer's clock");
    let mut d = seg(PEER_ISS + 1, ISS + 1, ACK);
    d.options.timestamps = Some((2000, 0));
    c.input(t(20), &d, b"new");
    assert_eq!(c.recv_queued(), 3);
    // An old duplicate (earlier timestamp) carrying data is dropped and
    // acknowledged; one without data is just dropped.
    out(&mut c, t(100));
    d.seq = PEER_ISS + 4;
    d.options.timestamps = Some((1500, 0));
    c.input(t(30), &d, b"old");
    assert_eq!(c.recv_queued(), 3);
    assert!(out(&mut c, t(30)).is_some());
    c.input(t(31), &d, b"");
    assert!(out(&mut c, t(31)).is_none());
}

#[test]
fn fast_retransmit_on_three_duplicate_acks() {
    let mut c = established(cfg());
    let data = [9u8; 5000];
    c.send(&data).unwrap();
    let mut segs = alloc::vec::Vec::new();
    while let Some((h, d)) = out(&mut c, t(20)) {
        segs.push((h, d.len()));
    }
    assert_eq!(segs.len(), 4, "initial window of 4 segments of 1000 (RFC 5681)");
    let una = ISS + 1;
    for i in 0..3 {
        c.input(t(30 + i), &seg(PEER_ISS + 1, una, ACK), &[]);
        if i < 2 {
            assert!(out(&mut c, t(30 + i)).is_none_or(|(h, _)| h.seq != una));
        }
    }
    assert_eq!(c.stats.fast_retransmits, 1);
    let (r, d) = out(&mut c, t(33)).unwrap();
    assert_eq!((r.seq, d.len()), (una, 1000), "the missing segment again");
    // The ACK for everything ends recovery with cwnd = ssthresh.
    c.input(t(40), &seg(PEER_ISS + 1, una + 4000, ACK), &[]);
    assert!(!c.in_recovery);
    assert_eq!(c.cwnd(), c.ssthresh);
}
