//! TCP (RFC 9293): one transmission control block per connection.
//!
//! The block is pure protocol state. The stack feeds it segments
//! (`Tcb::input`), runs its timers (`Tcb::on_timers`) and asks it for
//! segments to send (`Tcb::dispatch`); the socket layer moves bytes in and
//! out (`Tcb::send`, `Tcb::recv`).
//!
//! What is implemented, and where it comes from:
//! - The state machine and segment processing of RFC 9293 3.10, including
//!   simultaneous open and close, TIME-WAIT for 2 MSL, and the RFC 5961
//!   defences RFC 9293 recommends (challenge ACKs for in-window RSTs and
//!   SYNs, ACK range check).
//! - Retransmission timeout per RFC 6298 with Karn's algorithm (one timed
//!   segment per round trip, none while retransmitting).
//! - Congestion control per RFC 5681 (slow start, congestion avoidance,
//!   fast retransmit and fast recovery) with the NewReno modification of
//!   RFC 6582 for partial acknowledgements.
//! - Nagle's algorithm (RFC 9293 3.7.4, off with `nodelay`), sender and
//!   receiver silly window avoidance (RFC 9293 3.8.6.2), delayed ACKs
//!   (RFC 9293 3.8.6.3, RFC 5681 4.2: every second full segment, or after
//!   a delay), zero-window probing (RFC 9293 3.8.6.1).
//! - Options: MSS (RFC 9293 3.7.1), window scale and timestamps with PAWS
//!   (RFC 7323). SACK (RFC 2018) is not offered.
//! - Keepalives (RFC 9293 3.8.4, RFC 1122 4.2.3.6), off by default.
//! - Closing with unread data, or receiving data after close, resets the
//!   connection (RFC 1122 4.2.2.13).
//! - Urgent data is not supported: the urgent pointer is ignored and the
//!   data delivered in line (RFC 6093 recommends against using it).

pub(crate) mod buffer;
pub(crate) mod rtt;
pub(crate) mod seq;

mod input;
mod output;
mod timers;
mod user;

use crate::errno::Errno;
use crate::time::{Instant, earliest};
use crate::wire::tcp::{ACK, FIN, PSH, RST, SYN, TcpHeader, TcpOptions};
use alloc::collections::VecDeque;
use buffer::{Assembler, RingBuf};
use core::net::SocketAddr;
use core::time::Duration;
use rtt::Rtt;
use seq::Seq;

/// Connection states (RFC 9293 3.3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

/// TCP tunables, shared by all connections of a stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpConfig {
    /// Send buffer per connection (unacknowledged plus unsent bytes).
    pub send_buffer: usize,
    /// Receive buffer per connection; the advertised window never exceeds it.
    pub recv_buffer: usize,
    /// Disable Nagle's algorithm on new sockets (`TCP_NODELAY`).
    pub nodelay: bool,
    /// Longest an acknowledgement is delayed (RFC 1122 4.2.3.2: under 0.5 s).
    pub delayed_ack: Duration,
    /// RTO before the first measurement (RFC 6298 2.1: 1 s).
    pub initial_rto: Duration,
    /// Lower bound of the RTO (RFC 6298 2.4: 1 s; many stacks use 200 ms).
    pub min_rto: Duration,
    /// Upper bound of the RTO (RFC 6298 2.5: at least 60 s).
    pub max_rto: Duration,
    /// Retransmissions of a SYN or SYN-ACK before giving up.
    pub syn_retries: u32,
    /// Retransmissions of data before the connection fails with `ETIMEDOUT`.
    pub max_retransmits: u32,
    /// Maximum segment lifetime; TIME-WAIT lasts twice this.
    pub msl: Duration,
    /// How long a closed (orphaned) connection may sit in FIN-WAIT-2.
    pub fin_wait2_timeout: Duration,
    /// Idle time before the first keepalive probe (RFC 1122: at least 2 h).
    pub keepalive_idle: Duration,
    /// Time between unanswered keepalive probes.
    pub keepalive_interval: Duration,
    /// Unanswered keepalive probes before the connection fails.
    pub keepalive_probes: u32,
    /// Offer the window scale option (RFC 7323).
    pub window_scaling: bool,
    /// Offer the timestamps option (RFC 7323).
    pub timestamps: bool,
    /// Half-open connections per listening socket.
    pub syn_backlog: usize,
    /// Out-of-order ranges kept per connection.
    pub max_ooo_ranges: usize,
}

impl Default for TcpConfig {
    fn default() -> Self {
        TcpConfig {
            send_buffer: 128 * 1024,
            recv_buffer: 128 * 1024,
            nodelay: false,
            delayed_ack: Duration::from_millis(40),
            initial_rto: Duration::from_secs(1),
            min_rto: Duration::from_secs(1),
            max_rto: Duration::from_secs(60),
            syn_retries: 6,
            max_retransmits: 15,
            msl: Duration::from_secs(30),
            fin_wait2_timeout: Duration::from_secs(60),
            keepalive_idle: Duration::from_secs(2 * 3600),
            keepalive_interval: Duration::from_secs(75),
            keepalive_probes: 9,
            window_scaling: true,
            timestamps: true,
            syn_backlog: 128,
            max_ooo_ranges: 32,
        }
    }
}

/// Per-connection counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TcpStats {
    pub segs_in: u64,
    pub segs_out: u64,
    /// Segments sent again (timeouts, fast retransmits, MTU changes).
    pub retransmits: u64,
    /// Retransmission timer expiries.
    pub timeouts: u64,
    /// Fast retransmits entered on duplicate ACKs.
    pub fast_retransmits: u64,
    pub dup_acks_in: u64,
    pub window_probes: u64,
    pub keepalive_probes: u64,
    /// Pure ACKs sent (no data, SYN or FIN).
    pub acks_out: u64,
}

/// Default MSS when the peer sends none (RFC 9293 3.7.1; RFC 8200 for IPv6).
pub(crate) fn default_mss(v6: bool) -> usize {
    if v6 { 1220 } else { 536 }
}

/// Bytes the timestamps option adds to every segment.
const TS_OPTION_LEN: usize = 12;
/// Smallest MSS accepted from a peer or a "fragmentation needed" message.
const MIN_MSS: usize = 64;

/// Listening state: connections waiting to be accepted.
#[derive(Debug)]
pub(crate) struct ListenQueue {
    pub backlog: usize,
    /// Established connections not yet accepted, by socket slot.
    pub ready: VecDeque<usize>,
    /// Children still in SYN-RECEIVED.
    pub half_open: usize,
}

/// An ICMP error that concerns a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IcmpError {
    /// Unreachable. `hard` errors abort a connection that is still opening.
    Unreachable { errno: Errno, hard: bool },
    /// Fragmentation needed: the path takes packets of at most this size.
    PathMtu(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    Rst,
    Syn,
    SynAck,
    Data { seq: Seq, len: usize, fin: bool, kind: DataKind },
    Ack,
    Keepalive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DataKind {
    Normal,
    FastRetransmit,
    Probe,
}

/// A transmission control block.
#[derive(Debug)]
pub(crate) struct Tcb {
    pub state: TcpState,
    pub local: SocketAddr,
    pub remote: SocketAddr,
    pub cfg: TcpConfig,
    pub nodelay: bool,
    pub keepalive: bool,
    /// Path MSS limit from the interface MTU.
    mss_limit: usize,

    iss: Seq,
    snd_una: Seq,
    snd_nxt: Seq,
    /// Highest sequence number sent (snd_nxt goes back on a timeout).
    snd_max: Seq,
    snd_wnd: usize,
    snd_wl1: Seq,
    snd_wl2: Seq,
    max_snd_wnd: usize,
    snd_wscale: u8,
    sbuf: RingBuf,
    /// Sequence number of our FIN, once the user has closed the send side.
    fin_seq: Option<Seq>,

    irs: Seq,
    rcv_nxt: Seq,
    /// Right edge of the window we advertised last.
    rcv_adv: Seq,
    rcv_wscale: u8,
    rbuf: RingBuf,
    asm: Assembler,
    /// Sequence number of a FIN that arrived ahead of missing data.
    ooo_fin: Option<Seq>,
    pub fin_received: bool,
    /// Largest segment seen, for "every second full-sized segment".
    rcv_mss: usize,

    /// Send MSS: the peer's MSS option, clamped by our path.
    mss: usize,
    ws_enabled: bool,
    ts_enabled: bool,
    ts_recent: u32,
    ts_offset: u32,
    last_ack_sent: Seq,

    rtt: Rtt,
    rtx_at: Option<Instant>,
    retries: u32,
    rtt_probe: Option<(Seq, Instant)>,
    syn_retransmitted: bool,
    persist_at: Option<Instant>,
    persist_backoff: u32,
    /// Window probes sent without any ACK coming back.
    probes_unanswered: u32,
    probe_now: bool,
    delack_at: Option<Instant>,
    ack_now: bool,
    unacked_bytes: usize,
    timewait_at: Option<Instant>,
    keep_at: Option<Instant>,
    keep_sent: u32,
    keep_now: bool,
    last_send: Instant,

    cwnd: usize,
    ssthresh: usize,
    dup_acks: u32,
    recover: Seq,
    in_recovery: bool,
    fast_rtx: bool,

    /// Error reported by the next socket call (`SO_ERROR`).
    pub error: Option<Errno>,
    /// Last ICMP error, reported if the connection then times out.
    soft_error: Option<Errno>,
    /// The connection got as far as SYN-SENT or SYN-RECEIVED at some point.
    pub was_connected: bool,
    pub rd_shut: bool,
    pub wr_shut: bool,
    /// The user closed the socket; the connection finishes on its own.
    pub orphan: bool,
    rst_pending: bool,
    pub listen: Option<ListenQueue>,
    /// The listening socket this connection arrived on, until accepted.
    pub parent: Option<usize>,
    /// Already in the listener's queue of established connections.
    pub accept_queued: bool,
    pub stats: TcpStats,
    /// Per-socket IP options (`IP_TTL` / `IPV6_UNICAST_HOPS`, `IP_TOS`);
    /// `None` uses the stack's TTL.
    pub ttl: Option<u8>,
    pub tos: u8,
}

fn rcv_wscale_for(buffer: usize) -> u8 {
    let mut s = 0u8;
    while s < crate::wire::tcp::MAX_WSCALE && (buffer >> s) > 65535 {
        s += 1;
    }
    s
}

/// RFC 5681 3.1 initial window.
fn initial_window(mss: usize) -> usize {
    if mss > 2190 {
        2 * mss
    } else if mss > 1095 {
        3 * mss
    } else {
        4 * mss
    }
}

impl Tcb {
    /// A fresh block in CLOSED.
    pub fn new(cfg: TcpConfig, local: SocketAddr, now: Instant) -> Tcb {
        let rbuf_cap = cfg.recv_buffer.max(1);
        Tcb {
            state: TcpState::Closed,
            local,
            remote: local,
            cfg,
            nodelay: cfg.nodelay,
            keepalive: false,
            mss_limit: 536,
            iss: Seq(0),
            snd_una: Seq(0),
            snd_nxt: Seq(0),
            snd_max: Seq(0),
            snd_wnd: 0,
            snd_wl1: Seq(0),
            snd_wl2: Seq(0),
            max_snd_wnd: 0,
            snd_wscale: 0,
            sbuf: RingBuf::new(0),
            fin_seq: None,
            irs: Seq(0),
            rcv_nxt: Seq(0),
            rcv_adv: Seq(0),
            // Buffers are allocated when a connection starts, not for
            // every socket that is only ever bound or listening.
            rcv_wscale: rcv_wscale_for(rbuf_cap),
            rbuf: RingBuf::new(0),
            asm: Assembler::new(cfg.max_ooo_ranges),
            ooo_fin: None,
            fin_received: false,
            rcv_mss: 536,
            mss: 536,
            ws_enabled: false,
            ts_enabled: false,
            ts_recent: 0,
            ts_offset: 0,
            last_ack_sent: Seq(0),
            rtt: Rtt::new(cfg.initial_rto, cfg.min_rto, cfg.max_rto),
            rtx_at: None,
            retries: 0,
            rtt_probe: None,
            syn_retransmitted: false,
            persist_at: None,
            persist_backoff: 0,
            probes_unanswered: 0,
            probe_now: false,
            delack_at: None,
            ack_now: false,
            unacked_bytes: 0,
            timewait_at: None,
            keep_at: None,
            keep_sent: 0,
            keep_now: false,
            last_send: now,
            cwnd: 0,
            ssthresh: usize::MAX,
            dup_acks: 0,
            recover: Seq(0),
            in_recovery: false,
            fast_rtx: false,
            error: None,
            soft_error: None,
            was_connected: false,
            rd_shut: false,
            wr_shut: false,
            orphan: false,
            rst_pending: false,
            listen: None,
            parent: None,
            accept_queued: false,
            stats: TcpStats::default(),
            ttl: None,
            tos: 0,
        }
    }

    fn alloc_buffers(&mut self) {
        if self.rbuf.capacity() == 0 {
            self.rbuf = RingBuf::new(self.cfg.recv_buffer.max(1));
            self.sbuf = RingBuf::new(self.cfg.send_buffer.max(1));
        }
    }

    /// Change buffer sizes; only before the connection starts.
    pub fn set_buffers(&mut self, send: Option<usize>, recv: Option<usize>) -> bool {
        if self.rbuf.capacity() != 0 {
            return false;
        }
        if let Some(s) = send {
            self.cfg.send_buffer = s.max(1);
        }
        if let Some(r) = recv {
            self.cfg.recv_buffer = r.max(1);
            self.rcv_wscale = rcv_wscale_for(self.cfg.recv_buffer);
        }
        true
    }

    fn start(&mut self, iss: Seq, mss_limit: usize, now: Instant) {
        self.alloc_buffers();
        self.iss = iss;
        self.snd_una = iss;
        self.snd_nxt = iss;
        self.snd_max = iss;
        self.recover = iss;
        self.mss_limit = mss_limit.max(MIN_MSS);
        self.mss = default_mss(self.local.is_ipv6()).min(self.mss_limit);
        // RFC 7323 7.1: a per-connection offset keeps timestamps from
        // revealing the clock.
        self.ts_offset = iss.0.rotate_left(13) ^ 0x5bd1_e995;
        self.last_send = now;
        self.was_connected = true;
        if !self.cfg.window_scaling {
            self.rcv_wscale = 0;
        }
    }

    /// Active open (RFC 9293 3.10.1): go to SYN-SENT; `dispatch` sends the SYN.
    pub fn connect(&mut self, local: SocketAddr, remote: SocketAddr, iss: Seq, mss_limit: usize, now: Instant) {
        self.local = local;
        self.remote = remote;
        self.start(iss, mss_limit, now);
        self.state = TcpState::SynSent;
    }

    /// Passive open: become LISTEN.
    pub fn listen(&mut self, backlog: usize) {
        self.state = TcpState::Listen;
        match &mut self.listen {
            Some(q) => q.backlog = backlog,
            None => self.listen = Some(ListenQueue { backlog, ready: VecDeque::new(), half_open: 0 }),
        }
    }

    /// A new connection in SYN-RECEIVED for a SYN that reached a listener
    /// (RFC 9293 3.10.7.2).
    #[allow(clippy::too_many_arguments)]
    pub fn from_syn(
        cfg: TcpConfig,
        nodelay: bool,
        local: SocketAddr,
        remote: SocketAddr,
        h: &TcpHeader,
        iss: Seq,
        mss_limit: usize,
        now: Instant,
    ) -> Tcb {
        let mut t = Tcb::new(cfg, local, now);
        t.nodelay = nodelay;
        t.remote = remote;
        t.start(iss, mss_limit, now);
        t.state = TcpState::SynReceived;
        t.take_syn(h, now);
        t
    }

    /// Learn from a SYN: initial receive sequence, options, window.
    fn take_syn(&mut self, h: &TcpHeader, now: Instant) {
        self.irs = Seq(h.seq);
        self.rcv_nxt = self.irs + 1;
        self.last_ack_sent = self.rcv_nxt;
        let peer_mss = h.options.mss.map_or(default_mss(self.local.is_ipv6()), usize::from);
        self.mss = peer_mss.min(self.mss_limit).max(MIN_MSS);
        match h.options.wscale {
            Some(ws) if self.cfg.window_scaling => {
                self.ws_enabled = true;
                self.snd_wscale = ws;
            }
            _ => {
                self.ws_enabled = false;
                self.snd_wscale = 0;
                self.rcv_wscale = 0;
            }
        }
        match h.options.timestamps {
            Some((val, _)) if self.cfg.timestamps => {
                self.ts_enabled = true;
                self.ts_recent = val;
            }
            _ => self.ts_enabled = false,
        }
        // The window in a SYN is never scaled (RFC 7323 2.2).
        self.snd_wnd = usize::from(h.window);
        self.max_snd_wnd = self.snd_wnd;
        self.snd_wl1 = Seq(h.seq);
        self.snd_wl2 = Seq(h.ack);
        self.rcv_adv = self.rcv_nxt + self.rbuf.free().min(65535);
        self.cwnd = initial_window(self.eff_mss());
        self.last_send = now;
    }

    fn eff_mss(&self) -> usize {
        let opt = if self.ts_enabled { TS_OPTION_LEN } else { 0 };
        self.mss.saturating_sub(opt).max(1)
    }

    fn flight(&self) -> usize {
        self.snd_max.since(self.snd_una)
    }

    fn fin_acked(&self) -> bool {
        self.fin_seq.is_some_and(|f| self.snd_una > f)
    }

    fn synchronized(&self) -> bool {
        !matches!(self.state, TcpState::Closed | TcpState::Listen | TcpState::SynSent | TcpState::SynReceived)
    }
}

/// The RST answering a segment with an unacceptable ACK: `<SEQ=SEG.ACK><CTL=RST>`.
pub(crate) fn rst_for_ack(h: &TcpHeader) -> TcpHeader {
    TcpHeader { src_port: h.dst_port, dst_port: h.src_port, seq: h.ack, flags: RST, ..Default::default() }
}

/// The RST answering a segment for which no connection exists (RFC 9293
/// 3.10.7.1). `None` if the segment is itself a RST.
pub(crate) fn rst_for_closed(h: &TcpHeader, payload_len: usize) -> Option<TcpHeader> {
    if h.has(RST) {
        return None;
    }
    if h.has(ACK) {
        return Some(rst_for_ack(h));
    }
    let seg_len = payload_len + usize::from(h.has(SYN)) + usize::from(h.has(FIN));
    Some(TcpHeader {
        src_port: h.dst_port,
        dst_port: h.src_port,
        seq: 0,
        ack: h.seq.wrapping_add(seg_len as u32),
        flags: RST | ACK,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests;
