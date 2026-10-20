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

    // ---- user calls --------------------------------------------------

    /// Queue data; returns how much fit.
    pub fn send(&mut self, data: &[u8]) -> Result<usize, Errno> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        match self.state {
            TcpState::Closed if self.was_connected => Err(Errno::EPIPE),
            TcpState::Closed | TcpState::Listen => Err(Errno::ENOTCONN),
            TcpState::SynSent | TcpState::SynReceived if self.wr_shut => Err(Errno::EPIPE),
            TcpState::SynSent | TcpState::SynReceived => Err(Errno::EAGAIN),
            TcpState::Established | TcpState::CloseWait if !self.wr_shut => {
                if data.is_empty() {
                    return Ok(0);
                }
                match self.sbuf.push(data) {
                    0 => Err(Errno::EAGAIN),
                    n => Ok(n),
                }
            }
            _ => Err(Errno::EPIPE),
        }
    }

    /// Read received data. `Ok(0)` is end of stream.
    pub fn recv(&mut self, out: &mut [u8], peek: bool) -> Result<usize, Errno> {
        if !self.rbuf.is_empty() && !self.rd_shut {
            if out.is_empty() {
                return Ok(0);
            }
            let n = if peek { self.rbuf.peek(0, out) } else { self.rbuf.pop(out) };
            if !peek {
                self.maybe_window_update();
            }
            return Ok(n);
        }
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        match self.state {
            _ if self.rd_shut => Ok(0),
            TcpState::Closed if self.was_connected => Ok(0),
            TcpState::Closed | TcpState::Listen => Err(Errno::ENOTCONN),
            TcpState::SynSent | TcpState::SynReceived => Err(Errno::EAGAIN),
            _ if self.fin_received => Ok(0),
            _ => Err(Errno::EAGAIN),
        }
    }

    /// Bytes waiting to be read.
    pub fn recv_queued(&self) -> usize {
        self.rbuf.len()
    }

    /// Bytes not yet acknowledged by the peer (sent or not).
    pub fn send_queued(&self) -> usize {
        self.sbuf.len()
    }

    /// Ask for a window update when reading opened the window enough
    /// (receiver SWS avoidance, RFC 9293 3.8.6.2.2).
    fn maybe_window_update(&mut self) {
        if !matches!(self.state, TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2) {
            return;
        }
        let adv = self.rcv_adv.since(self.rcv_nxt);
        let free = self.rbuf.free();
        let threshold = (self.rbuf.capacity() / 2).min(self.mss_limit).max(1);
        if free >= adv + threshold {
            self.ack_now = true;
        }
    }

    /// Shut down the sending side: a FIN follows the queued data.
    pub fn shutdown_write(&mut self) {
        self.wr_shut = true;
        match self.state {
            TcpState::Established => {
                self.state = TcpState::FinWait1;
                self.queue_fin();
            }
            TcpState::CloseWait => {
                self.state = TcpState::LastAck;
                self.queue_fin();
            }
            // SYN-SENT and SYN-RECEIVED: the FIN goes out once established.
            _ => {}
        }
    }

    fn queue_fin(&mut self) {
        if self.fin_seq.is_none() {
            self.fin_seq = Some(self.snd_una + self.sbuf.len());
        }
    }

    /// Shut down the receiving side: reads return end of stream, later data
    /// is acknowledged and dropped.
    pub fn shutdown_read(&mut self) {
        self.rd_shut = true;
        let n = self.rbuf.len();
        self.rbuf.consume(n);
    }

    /// The user closed the socket. Returns true if the block can be freed
    /// right away; otherwise the connection finishes on its own (orphan).
    pub fn close(&mut self, now: Instant) -> bool {
        match self.state {
            TcpState::Closed | TcpState::Listen | TcpState::SynSent => {
                self.state = TcpState::Closed;
                true
            }
            TcpState::SynReceived | TcpState::Established | TcpState::CloseWait => {
                self.orphan = true;
                if !self.rbuf.is_empty() {
                    // RFC 1122 4.2.2.13: unread data is lost, so reset.
                    self.abort();
                } else {
                    self.shutdown_write();
                }
                false
            }
            TcpState::FinWait2 => {
                self.orphan = true;
                self.timewait_at = Some(now + self.cfg.fin_wait2_timeout);
                false
            }
            _ => {
                self.orphan = true;
                false
            }
        }
    }

    /// Send a RST and drop the connection (RFC 9293 3.10.5 ABORT).
    pub fn abort(&mut self) {
        if matches!(self.state, TcpState::Closed | TcpState::Listen | TcpState::SynSent | TcpState::TimeWait) {
            self.become_closed();
        } else {
            self.rst_pending = true;
        }
    }

    fn become_closed(&mut self) {
        self.state = TcpState::Closed;
        self.rtx_at = None;
        self.persist_at = None;
        self.delack_at = None;
        self.timewait_at = None;
        self.keep_at = None;
        self.ack_now = false;
        self.fast_rtx = false;
        self.probe_now = false;
        self.keep_now = false;
    }

    /// Fail the connection with `e`, reported to the user.
    fn fail(&mut self, e: Errno) {
        self.error = Some(e);
        self.become_closed();
    }

    /// Enable or disable keepalives (`SO_KEEPALIVE`).
    pub fn set_keepalive(&mut self, on: bool, now: Instant) {
        self.keepalive = on;
        self.keep_sent = 0;
        self.keep_at = if on && self.synchronized() { Some(now + self.cfg.keepalive_idle) } else { None };
    }

    /// Data can be read, or a read would not block.
    pub fn readable(&self) -> bool {
        !self.rbuf.is_empty()
            || self.fin_received
            || self.rd_shut
            || self.error.is_some()
            || (self.state == TcpState::Closed && self.was_connected)
            || self.listen.as_ref().is_some_and(|l| !l.ready.is_empty())
    }

    /// A write would not block.
    pub fn writable(&self) -> bool {
        match self.state {
            TcpState::Established | TcpState::CloseWait => self.wr_shut || self.sbuf.free() > 0,
            TcpState::Closed => self.was_connected,
            TcpState::SynSent | TcpState::SynReceived | TcpState::Listen => self.error.is_some(),
            _ => true,
        }
    }

    /// Both directions are finished (or never started).
    pub fn hangup(&self) -> bool {
        match self.state {
            TcpState::Closed => true,
            TcpState::Listen | TcpState::SynSent | TcpState::SynReceived => false,
            _ => (self.wr_shut || self.fin_seq.is_some()) && (self.fin_received || self.rd_shut),
        }
    }

    /// Smoothed round-trip time, if measured.
    pub fn srtt(&self) -> Option<Duration> {
        self.rtt.srtt()
    }

    /// Congestion window in bytes, for tests and statistics.
    pub fn cwnd(&self) -> usize {
        self.cwnd
    }

    /// The scale factors in use (send, receive), if negotiated.
    pub fn window_scales(&self) -> Option<(u8, u8)> {
        self.ws_enabled.then_some((self.snd_wscale, self.rcv_wscale))
    }

    /// Effective send MSS.
    pub fn send_mss(&self) -> usize {
        self.eff_mss()
    }

    /// True if the timestamps option is in use.
    pub fn timestamps_enabled(&self) -> bool {
        self.ts_enabled
    }

    // ---- input -------------------------------------------------------

    /// Window within which segments are acceptable: what we advertised, or
    /// more if reading has freed space since.
    fn accept_window(&self) -> usize {
        self.rcv_adv.since(self.rcv_nxt).max(self.rbuf.free())
    }

    /// Process an arriving segment (RFC 9293 3.10.7.3 and 3.10.7.4) for a
    /// connection in SYN-SENT or later. Returns a RST to send for an
    /// unacceptable ACK in SYN-SENT or SYN-RECEIVED.
    pub fn input(&mut self, now: Instant, h: &TcpHeader, payload: &[u8]) -> Option<TcpHeader> {
        self.stats.segs_in += 1;
        match self.state {
            TcpState::Closed | TcpState::Listen => return None,
            TcpState::SynSent => return self.input_syn_sent(now, h),
            _ => {}
        }
        let mut seg_seq = Seq(h.seq);
        let mut syn = h.has(SYN);
        if syn && h.has(ACK) && self.state == TcpState::SynReceived && seg_seq == self.irs {
            // Simultaneous open: the peer's SYN-ACK repeats the SYN we
            // already took; what matters is its ACK.
            syn = false;
            seg_seq = seg_seq + 1;
            self.ack_now = true;
        }
        let seg_len = payload.len() + usize::from(syn) + usize::from(h.has(FIN));

        // PAWS (RFC 7323 5.3): a timestamp older than the last one seen
        // marks an old duplicate. It is acknowledged if it occupies
        // sequence space; a reordered pure ACK is just dropped, since
        // answering ACKs with ACKs only adds traffic.
        if self.ts_enabled
            && !h.has(RST)
            && let Some((val, _)) = h.options.timestamps
            && (val.wrapping_sub(self.ts_recent) as i32) < 0
        {
            if seg_len > 0 {
                self.ack_now = true;
            }
            return None;
        }

        // First check: sequence number (RFC 9293 3.10.7.4).
        let wnd = self.accept_window();
        let in_wnd = |s: Seq| s.in_range(self.rcv_nxt, self.rcv_nxt + wnd);
        let mut ctl_only = false;
        let acceptable = match (seg_len, wnd) {
            (0, 0) => seg_seq == self.rcv_nxt,
            (0, _) => in_wnd(seg_seq),
            (_, 0) => {
                // Zero window: data is not acceptable (and is acknowledged,
                // which answers window probes), but ACKs and RSTs at the
                // right sequence number still are.
                ctl_only = seg_seq == self.rcv_nxt;
                if ctl_only && !h.has(RST) {
                    self.ack_now = true;
                }
                ctl_only
            }
            _ => in_wnd(seg_seq) || in_wnd(seg_seq + (seg_len - 1)),
        };
        if !acceptable {
            if self.state == TcpState::SynReceived && h.has(SYN) && !h.has(ACK) && seg_seq == self.irs {
                // The peer did not get our SYN-ACK: send it again.
                self.snd_nxt = self.iss;
                return None;
            }
            if self.state == TcpState::TimeWait && h.has(FIN) {
                self.timewait_at = Some(now + 2 * self.cfg.msl);
            }
            if !h.has(RST) {
                self.ack_now = true;
            }
            return None;
        }

        // Trim to the window: drop what was already received and what does
        // not fit.
        let mut seq = seg_seq;
        let mut data = if ctl_only { &[][..] } else { payload };
        let mut fin = h.has(FIN) && !ctl_only;
        if seq < self.rcv_nxt {
            let mut skip = self.rcv_nxt.since(seq);
            if syn {
                syn = false;
                skip -= 1;
                seq = seq + 1;
            }
            let d = skip.min(data.len());
            data = &data[d..];
            seq = seq + d;
            if skip > d {
                fin = false;
            }
        }
        let room = (self.rcv_nxt + wnd).since(seq);
        if data.len() > room {
            // The sender overran the window: tell it where it stands now.
            data = &data[..room];
            fin = false;
            self.ack_now = true;
        }

        // Second: RST (RFC 5961 3.2 as adopted by RFC 9293).
        if h.has(RST) {
            if seg_seq == self.rcv_nxt {
                match self.state {
                    TcpState::SynReceived if self.parent.is_some() => self.become_closed(),
                    TcpState::SynReceived => self.fail(Errno::ECONNREFUSED),
                    TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2 | TcpState::CloseWait => {
                        self.fail(Errno::ECONNRESET)
                    }
                    _ => self.become_closed(),
                }
            } else {
                self.ack_now = true; // challenge ACK
            }
            return None;
        }

        // Fourth: a SYN in the window gets a challenge ACK (RFC 5961 4.2).
        if syn {
            self.ack_now = true;
            return None;
        }

        // Fifth: the ACK field.
        if !h.has(ACK) {
            return None;
        }
        let ack = Seq(h.ack);
        if self.state == TcpState::SynReceived {
            if ack > self.snd_una && ack <= self.snd_max {
                self.established(now);
                self.snd_wnd = usize::from(h.window) << self.snd_wscale;
                self.snd_wl1 = seg_seq;
                self.snd_wl2 = ack;
            } else {
                return Some(rst_for_ack(h));
            }
        }
        if !self.process_ack(now, h, seg_seq, data.len()) {
            return None;
        }
        self.probes_unanswered = 0;
        match self.state {
            TcpState::FinWait1 if self.fin_acked() => {
                self.state = TcpState::FinWait2;
                if self.orphan {
                    self.timewait_at = Some(now + self.cfg.fin_wait2_timeout);
                }
            }
            TcpState::Closing if self.fin_acked() => self.enter_time_wait(now),
            TcpState::LastAck if self.fin_acked() => {
                self.become_closed();
                return None;
            }
            _ => {}
        }

        // Seventh: segment text.
        let mut fin_ok = fin;
        if !data.is_empty() {
            match self.state {
                TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2 => {
                    if self.orphan {
                        // Data after close (RFC 1122 4.2.2.13).
                        self.abort();
                        return None;
                    }
                    fin_ok = self.accept_data(now, seq, data) && fin;
                }
                _ => fin_ok = false,
            }
        }

        // Timestamps: remember the peer's clock (RFC 7323 4.3).
        if self.ts_enabled
            && let Some((val, _)) = h.options.timestamps
            && seg_seq <= self.last_ack_sent
            && (val.wrapping_sub(self.ts_recent) as i32) >= 0
        {
            self.ts_recent = val;
        }

        // Eighth: FIN.
        if fin_ok {
            let fin_at = seq + data.len();
            if fin_at == self.rcv_nxt {
                self.process_fin(now);
            } else if fin_at > self.rcv_nxt {
                self.ooo_fin = Some(fin_at);
                self.ack_now = true;
            }
        }
        if self.keepalive && self.synchronized() {
            self.keep_sent = 0;
            self.keep_at = Some(now + self.cfg.keepalive_idle);
        }
        self.update_persist(now);
        None
    }

    fn input_syn_sent(&mut self, now: Instant, h: &TcpHeader) -> Option<TcpHeader> {
        let ack = Seq(h.ack);
        if h.has(ACK) && (ack <= self.iss || ack > self.snd_max) {
            return if h.has(RST) { None } else { Some(rst_for_ack(h)) };
        }
        if h.has(RST) {
            if h.has(ACK) {
                self.fail(Errno::ECONNREFUSED);
            }
            return None;
        }
        if !h.has(SYN) {
            return None;
        }
        self.take_syn(h, now);
        if h.has(ACK) {
            self.snd_una = ack;
        }
        if self.snd_una > self.iss {
            if let Some((_, t)) = self.rtt_probe.take() {
                self.rtt.sample(now.since(t));
            } else if self.syn_retransmitted {
                self.rtt.at_least(Duration::from_secs(3));
            }
            self.rtt.reset_backoff();
            self.retries = 0;
            self.rtx_at = None;
            self.established(now);
            self.ack_now = true;
        } else {
            // Simultaneous open: answer with SYN-ACK.
            self.state = TcpState::SynReceived;
            self.snd_nxt = self.iss;
        }
        None
    }

    fn established(&mut self, now: Instant) {
        self.state = TcpState::Established;
        if self.keepalive {
            self.keep_at = Some(now + self.cfg.keepalive_idle);
        }
        if self.wr_shut {
            self.shutdown_write();
        }
    }

    /// The ACK field (RFC 9293 3.10.7.4 fifth step). Returns false if the
    /// segment is to be dropped.
    fn process_ack(&mut self, now: Instant, h: &TcpHeader, seg_seq: Seq, data_len: usize) -> bool {
        let ack = Seq(h.ack);
        if ack > self.snd_max || ack < self.snd_una - self.max_snd_wnd.max(1) {
            // RFC 5961 5.2: an ACK outside the valid range gets an ACK back.
            self.ack_now = true;
            return false;
        }
        let new_wnd = usize::from(h.window) << self.snd_wscale;
        let ack_in_range = ack >= self.snd_una;
        let acked = ack.since(self.snd_una);
        let is_dup = acked == 0
            && data_len == 0
            && !h.has(SYN)
            && !h.has(FIN)
            && new_wnd == self.snd_wnd
            && self.snd_max > self.snd_una;

        if acked > 0 {
            let mut data_acked = acked;
            if self.snd_una == self.iss {
                data_acked -= 1; // our SYN
            }
            let from_buf = data_acked.min(self.sbuf.len());
            self.sbuf.consume(from_buf);
            self.snd_una = ack;
            if self.snd_nxt < self.snd_una {
                self.snd_nxt = self.snd_una;
            }
            if let Some((s, t)) = self.rtt_probe
                && ack >= s
            {
                self.rtt.sample(now.since(t));
                self.rtt_probe = None;
            }
            self.rtt.reset_backoff();
            self.retries = 0;
            self.on_new_ack(acked);
            self.rtx_at = if self.snd_una == self.snd_max { None } else { Some(now + self.rtt.rto()) };
        }

        // Window update (RFC 9293 3.10.7.4).
        if ack_in_range && (self.snd_wl1 < seg_seq || (self.snd_wl1 == seg_seq && self.snd_wl2 <= ack)) {
            self.snd_wnd = new_wnd;
            self.snd_wl1 = seg_seq;
            self.snd_wl2 = ack;
            self.max_snd_wnd = self.max_snd_wnd.max(new_wnd);
        }

        if is_dup {
            self.stats.dup_acks_in += 1;
            self.dup_acks += 1;
            self.on_dup_ack();
        } else if acked > 0 {
            self.dup_acks = 0;
        }
        true
    }

    /// Congestion control on an ACK of new data (RFC 5681 3.1, RFC 6582 3.2).
    fn on_new_ack(&mut self, acked: usize) {
        let mss = self.eff_mss();
        if self.in_recovery {
            if self.snd_una >= self.recover {
                self.in_recovery = false;
                self.cwnd = self.ssthresh.min(self.flight().max(mss) + mss);
            } else {
                // Partial ACK: retransmit the next hole, deflate the window.
                self.fast_rtx = true;
                self.cwnd = self.cwnd.saturating_sub(acked) + if acked >= mss { mss } else { 0 };
                self.cwnd = self.cwnd.max(mss);
            }
        } else if self.cwnd < self.ssthresh {
            self.cwnd += acked.min(mss);
        } else {
            self.cwnd += (mss * mss / self.cwnd.max(1)).max(1);
        }
        self.cwnd = self.cwnd.min(1 << 30);
    }

    /// Duplicate ACKs: fast retransmit on the third, then inflate.
    fn on_dup_ack(&mut self) {
        let mss = self.eff_mss();
        if self.in_recovery {
            self.cwnd += mss;
        } else if self.dup_acks == 3 && self.snd_una > self.recover {
            self.ssthresh = (self.flight() / 2).max(2 * mss);
            self.recover = self.snd_max;
            self.in_recovery = true;
            self.fast_rtx = true;
            self.cwnd = self.ssthresh + 3 * mss;
            self.stats.fast_retransmits += 1;
        }
    }

    /// Put in-window data into the receive buffer. Returns true if all of
    /// it was stored (so a FIN right after it counts).
    fn accept_data(&mut self, now: Instant, seq: Seq, data: &[u8]) -> bool {
        self.rcv_mss = self.rcv_mss.max(data.len());
        let off = seq.since(self.rcv_nxt);
        if self.rd_shut {
            // Acknowledge and drop in-order data.
            if off == 0 {
                self.rcv_nxt = self.rcv_nxt + data.len();
            }
            self.ack_now = true;
            return true;
        }
        let n = self.rbuf.write_at(off, data);
        if off == 0 {
            let had_gap = !self.asm.is_empty();
            self.rbuf.commit(n);
            self.asm.advance(n);
            let extra = self.asm.take_front();
            self.rbuf.commit(extra);
            self.rcv_nxt = self.rcv_nxt + n + extra;
            if let Some(f) = self.ooo_fin
                && f == self.rcv_nxt
            {
                self.ooo_fin = None;
                self.process_fin(now);
            }
            self.unacked_bytes += n + extra;
            if had_gap || self.unacked_bytes >= 2 * self.rcv_mss {
                self.ack_now = true;
            } else if self.delack_at.is_none() {
                self.delack_at = Some(now + self.cfg.delayed_ack);
            }
        } else {
            // Out of order: remember it and send a duplicate ACK now
            // (RFC 5681 4.2).
            if n > 0 && !self.asm.add(off, n) {
                self.ack_now = true;
                return false;
            }
            self.ack_now = true;
        }
        n == data.len()
    }

    fn process_fin(&mut self, now: Instant) {
        if self.fin_received {
            return;
        }
        self.rcv_nxt = self.rcv_nxt + 1;
        self.fin_received = true;
        self.ack_now = true;
        match self.state {
            TcpState::SynReceived | TcpState::Established => self.state = TcpState::CloseWait,
            TcpState::FinWait1 => {
                if self.fin_acked() {
                    self.enter_time_wait(now);
                } else {
                    self.state = TcpState::Closing;
                }
            }
            TcpState::FinWait2 => self.enter_time_wait(now),
            _ => {}
        }
    }

    fn enter_time_wait(&mut self, now: Instant) {
        self.state = TcpState::TimeWait;
        self.rtx_at = None;
        self.persist_at = None;
        self.keep_at = None;
        self.timewait_at = Some(now + 2 * self.cfg.msl);
    }

    /// Start or stop the persist timer: running while the peer's window is
    /// closed and data waits with nothing in flight.
    fn update_persist(&mut self, now: Instant) {
        let waiting = self.unsent() > 0 && self.snd_wnd == 0 && self.snd_nxt == self.snd_una;
        if waiting && self.synchronized() {
            if self.persist_at.is_none() {
                self.persist_at = Some(now + self.persist_interval());
            }
        } else {
            self.persist_at = None;
            self.persist_backoff = 0;
            self.probe_now = false;
        }
    }

    fn persist_interval(&self) -> Duration {
        let base = self.rtt.rto();
        let d = base.checked_mul(1u32 << self.persist_backoff.min(16)).unwrap_or(self.cfg.max_rto);
        d.min(self.cfg.max_rto)
    }

    fn unsent(&self) -> usize {
        if self.snd_una == self.iss {
            return 0;
        }
        self.sbuf.len().saturating_sub(self.snd_nxt.since(self.snd_una))
    }

    /// An ICMP error quoting a segment with sequence number `seq` (RFC 5927:
    /// only if it is in flight).
    pub fn icmp_error(&mut self, seq: Seq, err: IcmpError) {
        if !(seq >= self.snd_una && seq < self.snd_max) {
            return;
        }
        match err {
            IcmpError::PathMtu(mtu) => {
                let ip = if self.local.is_ipv6() { 40 } else { 20 };
                let mss = mtu.saturating_sub(ip + 20).max(MIN_MSS);
                if mss < self.mss {
                    self.mss = mss;
                    self.mss_limit = self.mss_limit.min(mss);
                    // Send what is in flight again, in smaller pieces.
                    self.snd_nxt = self.snd_una;
                    self.rtt_probe = None;
                }
            }
            IcmpError::Unreachable { errno, hard } => {
                if hard && matches!(self.state, TcpState::SynSent | TcpState::SynReceived) && self.parent.is_none() {
                    self.fail(errno);
                } else {
                    self.soft_error = Some(errno);
                }
            }
        }
    }

    /// Neighbour resolution for the peer failed.
    pub fn unreachable(&mut self, errno: Errno) {
        self.soft_error = Some(errno);
    }

    // ---- timers ------------------------------------------------------

    /// Run expired timers.
    pub fn on_timers(&mut self, now: Instant) {
        if let Some(t) = self.timewait_at
            && now >= t
            && matches!(self.state, TcpState::TimeWait | TcpState::FinWait2)
        {
            self.become_closed();
            return;
        }
        if let Some(t) = self.rtx_at
            && now >= t
        {
            self.on_retransmit_timeout(now);
        }
        if let Some(t) = self.persist_at
            && now >= t
        {
            self.probes_unanswered += 1;
            if self.probes_unanswered > self.cfg.max_retransmits {
                // RFC 1122 4.2.2.17 keeps probing while the receiver
                // answers; one that has gone silent fails like a timeout.
                self.fail(self.soft_error.unwrap_or(Errno::ETIMEDOUT));
                return;
            }
            self.probe_now = true;
            self.persist_backoff += 1;
            self.persist_at = Some(now + self.persist_interval());
        }
        if let Some(t) = self.delack_at
            && now >= t
        {
            self.delack_at = None;
            self.ack_now = true;
        }
        if let Some(t) = self.keep_at
            && now >= t
        {
            if !self.keepalive || !matches!(self.state, TcpState::Established | TcpState::CloseWait) {
                self.keep_at = None;
            } else if self.keep_sent >= self.cfg.keepalive_probes {
                self.fail(self.soft_error.unwrap_or(Errno::ETIMEDOUT));
            } else {
                self.keep_sent += 1;
                self.keep_now = true;
                self.keep_at = Some(now + self.cfg.keepalive_interval);
            }
        }
    }

    fn on_retransmit_timeout(&mut self, now: Instant) {
        self.retries += 1;
        let opening = matches!(self.state, TcpState::SynSent | TcpState::SynReceived);
        let limit = if opening { self.cfg.syn_retries } else { self.cfg.max_retransmits };
        if self.retries > limit {
            if self.parent.is_some() && opening {
                self.become_closed();
            } else {
                self.fail(self.soft_error.unwrap_or(Errno::ETIMEDOUT));
            }
            return;
        }
        self.stats.timeouts += 1;
        if opening {
            self.syn_retransmitted = true;
        } else {
            // RFC 5681 3.1 (equation 4) and the loss window.
            let mss = self.eff_mss();
            self.ssthresh = (self.flight() / 2).max(2 * mss);
            self.cwnd = mss;
            self.in_recovery = false;
            self.recover = self.snd_max;
            self.dup_acks = 0;
            self.fast_rtx = false;
        }
        // Go back: everything from snd_una is sent again.
        self.snd_nxt = self.snd_una;
        self.rtt_probe = None;
        self.rtt.back_off();
        self.rtx_at = Some(now + self.rtt.rto());
        if self.synchronized() && self.snd_wnd == 0 && self.fin_seq != Some(self.snd_una) {
            // Nothing may be sent into a closed window: probe instead.
            self.rtx_at = None;
            self.retries = 0;
            self.update_persist(now);
        }
    }

    /// The earliest running timer.
    pub fn timer_deadline(&self) -> Option<Instant> {
        let mut t = earliest(self.rtx_at, self.persist_at);
        t = earliest(t, self.delack_at);
        t = earliest(t, self.timewait_at);
        if self.keepalive {
            t = earliest(t, self.keep_at);
        }
        t
    }

    /// True if a SYN with sequence number `seq` may end TIME-WAIT and open a
    /// new connection (RFC 1122 4.2.2.13: it is above everything seen).
    pub fn time_wait_accepts(&self, seq: Seq) -> bool {
        self.state == TcpState::TimeWait && seq > self.rcv_nxt
    }

    /// True if [`dispatch`](Self::dispatch) would send something now.
    pub fn wants_to_send(&self) -> bool {
        self.plan().is_some()
    }

    // ---- output ------------------------------------------------------

    fn plan(&self) -> Option<Plan> {
        if self.rst_pending {
            return Some(Plan::Rst);
        }
        match self.state {
            TcpState::Closed | TcpState::Listen => return None,
            TcpState::SynSent => return (self.snd_nxt == self.iss).then_some(Plan::Syn),
            TcpState::SynReceived => return (self.snd_nxt == self.iss).then_some(Plan::SynAck),
            TcpState::TimeWait => return self.ack_now.then_some(Plan::Ack),
            _ => {}
        }
        let mss = self.eff_mss();
        let buffered = self.sbuf.len();
        if self.fast_rtx && self.snd_max > self.snd_una {
            let len = buffered.min(mss);
            let fin = self.fin_seq.is_some_and(|f| self.snd_una + len == f && self.snd_max > f);
            if len > 0 || fin {
                return Some(Plan::Data { seq: self.snd_una, len, fin, kind: DataKind::FastRetransmit });
            }
        }
        let off = self.snd_nxt.since(self.snd_una);
        let unsent = buffered.saturating_sub(off);
        let in_flight = self.snd_nxt.since(self.snd_una);
        let usable = self.snd_wnd.min(self.cwnd).saturating_sub(in_flight);
        let len = unsent.min(usable).min(mss);
        let fin = self.fin_seq == Some(self.snd_nxt + len);
        if self.probe_now && len == 0 && unsent > 0 {
            return Some(Plan::Data { seq: self.snd_nxt, len: 1, fin: false, kind: DataKind::Probe });
        }
        if len > 0 {
            let mut ok = len == mss || in_flight == 0 || fin;
            if !ok && len == unsent {
                // Everything left fits: Nagle holds it back while data is
                // unacknowledged (RFC 9293 3.7.4).
                ok = self.nodelay;
            } else if !ok {
                // Window-limited small segment: sender SWS avoidance.
                ok = len >= self.max_snd_wnd / 2;
            }
            if ok {
                return Some(Plan::Data { seq: self.snd_nxt, len, fin, kind: DataKind::Normal });
            }
        } else if fin {
            return Some(Plan::Data { seq: self.snd_nxt, len: 0, fin: true, kind: DataKind::Normal });
        }
        if self.keep_now {
            return Some(Plan::Keepalive);
        }
        self.ack_now.then_some(Plan::Ack)
    }

    /// The window to advertise, updating the right edge we remember. The
    /// edge only moves right, and only by a useful amount (receiver SWS
    /// avoidance).
    fn advertise(&mut self) -> u16 {
        let free = self.rbuf.free();
        let cur = self.rcv_adv.since(self.rcv_nxt);
        let threshold = (self.rbuf.capacity() / 2).min(self.mss_limit).max(1);
        let wnd = if free >= cur + threshold || free == self.rbuf.capacity() { free } else { cur };
        let scaled = (wnd >> self.rcv_wscale).min(65535);
        let edge = self.rcv_nxt + (scaled << self.rcv_wscale);
        self.rcv_adv = self.rcv_adv.max(edge);
        scaled as u16
    }

    fn timestamps(&self, now: Instant) -> Option<(u32, u32)> {
        self.ts_enabled.then(|| ((now.millis() as u32).wrapping_add(self.ts_offset), self.ts_recent))
    }

    fn header(&self, seq: Seq, flags: u8, window: u16) -> TcpHeader {
        TcpHeader {
            src_port: self.local.port(),
            dst_port: self.remote.port(),
            seq: seq.0,
            ack: if flags & ACK != 0 { self.rcv_nxt.0 } else { 0 },
            flags,
            window,
            urgent: 0,
            options: TcpOptions::default(),
        }
    }

    fn syn_options(&self, now: Instant, syn_ack: bool) -> TcpOptions {
        let ws = if syn_ack { self.ws_enabled } else { self.cfg.window_scaling };
        let ts = if syn_ack { self.ts_enabled } else { self.cfg.timestamps };
        TcpOptions {
            mss: Some(self.mss_limit.min(65535) as u16),
            wscale: ws.then_some(self.rcv_wscale),
            sack_permitted: false,
            timestamps: ts.then(|| ((now.millis() as u32).wrapping_add(self.ts_offset), self.ts_recent)),
        }
    }

    /// Produce the next segment to send, its payload copied to `buf`
    /// (which must hold at least the send MSS). Returns the header and
    /// payload length.
    pub fn dispatch(&mut self, now: Instant, buf: &mut [u8]) -> Option<(TcpHeader, usize)> {
        if self.state != TcpState::SynSent
            && self.snd_max == self.snd_una
            && now.since(self.last_send) > self.rtt.rto()
            && self.cwnd > 0
        {
            // Restart window after an idle period (RFC 5681 4.1).
            self.cwnd = self.cwnd.min(initial_window(self.eff_mss()));
        }
        let plan = self.plan()?;
        let (h, len) = match plan {
            Plan::Rst => {
                self.rst_pending = false;
                let h = self.header(self.snd_max, RST | ACK, 0);
                self.become_closed();
                (h, 0)
            }
            Plan::Syn => {
                let mut h = self.header(self.iss, SYN, self.rbuf.free().min(65535) as u16);
                h.options = self.syn_options(now, false);
                self.sent_seq_space(now, self.iss, 1);
                (h, 0)
            }
            Plan::SynAck => {
                let mut h = self.header(self.iss, SYN | ACK, self.rbuf.free().min(65535) as u16);
                h.options = self.syn_options(now, true);
                self.sent_seq_space(now, self.iss, 1);
                self.ack_sent();
                (h, 0)
            }
            Plan::Data { seq, len, fin, kind } => {
                let off = seq.since(self.snd_una);
                let room = len.min(buf.len());
                let n = self.sbuf.peek(off, &mut buf[..room]);
                let mut flags = ACK;
                if fin {
                    flags |= FIN;
                }
                if n > 0 && off + n == self.sbuf.len() {
                    flags |= PSH;
                }
                let window = self.advertise();
                let mut h = self.header(seq, flags, window);
                h.options.timestamps = self.timestamps(now);
                let seq_len = n + usize::from(fin);
                if seq < self.snd_max {
                    self.stats.retransmits += 1;
                    self.rtt_probe = None;
                }
                match kind {
                    DataKind::Normal => self.sent_seq_space(now, seq, seq_len),
                    DataKind::FastRetransmit => {
                        self.fast_rtx = false;
                        self.snd_max = self.snd_max.max(seq + seq_len);
                    }
                    DataKind::Probe => {
                        self.probe_now = false;
                        self.stats.window_probes += 1;
                        self.snd_max = self.snd_max.max(seq + seq_len);
                    }
                }
                self.ack_sent();
                (h, n)
            }
            Plan::Ack => {
                let window = self.advertise();
                let mut h = self.header(self.snd_max, ACK, window);
                h.options.timestamps = self.timestamps(now);
                self.ack_sent();
                self.stats.acks_out += 1;
                (h, 0)
            }
            Plan::Keepalive => {
                self.keep_now = false;
                self.stats.keepalive_probes += 1;
                let window = self.advertise();
                let mut h = self.header(self.snd_una - 1, ACK, window);
                h.options.timestamps = self.timestamps(now);
                (h, 0)
            }
        };
        self.stats.segs_out += 1;
        self.last_send = now;
        self.update_persist(now);
        Some((h, len))
    }

    /// Bookkeeping after sending `len` units of sequence space at `seq`.
    fn sent_seq_space(&mut self, now: Instant, seq: Seq, len: usize) {
        let end = seq + len;
        let new_data = end > self.snd_max;
        if new_data && self.rtt_probe.is_none() && seq >= self.snd_max {
            self.rtt_probe = Some((end, now));
        }
        self.snd_nxt = end;
        self.snd_max = self.snd_max.max(end);
        if self.rtx_at.is_none() && len > 0 {
            self.rtx_at = Some(now + self.rtt.rto());
        }
    }

    fn ack_sent(&mut self) {
        self.ack_now = false;
        self.delack_at = None;
        self.unacked_bytes = 0;
        self.last_ack_sent = self.rcv_nxt;
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
