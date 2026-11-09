//! Segment input: the RFC 9293 "SEGMENT ARRIVES" processing.

use super::*;

impl Tcb {
    /// Window within which segments are acceptable: what we advertised, or
    /// more if reading has freed space since.
    pub(super) fn accept_window(&self) -> usize {
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

    pub(super) fn input_syn_sent(&mut self, now: Instant, h: &TcpHeader) -> Option<TcpHeader> {
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

    pub(super) fn established(&mut self, now: Instant) {
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
    pub(super) fn process_ack(&mut self, now: Instant, h: &TcpHeader, seg_seq: Seq, data_len: usize) -> bool {
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
    pub(super) fn on_new_ack(&mut self, acked: usize) {
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
    pub(super) fn on_dup_ack(&mut self) {
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
    pub(super) fn accept_data(&mut self, now: Instant, seq: Seq, data: &[u8]) -> bool {
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
            // A peer on this host is acknowledged at once: the data a
            // sender's Nagle holds back for the acknowledgement then
            // follows in the same poll, so small writes over loopback are
            // all in the receiver's queue when the writes return (Linux
            // does that: recvfrom's spec sheet).
            let local = self.remote.ip().is_loopback() || self.remote.ip() == self.local.ip();
            if had_gap || local || self.unacked_bytes >= 2 * self.rcv_mss {
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

    pub(super) fn process_fin(&mut self, now: Instant) {
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

    pub(super) fn enter_time_wait(&mut self, now: Instant) {
        self.state = TcpState::TimeWait;
        self.rtx_at = None;
        self.persist_at = None;
        self.keep_at = None;
        self.timewait_at = Some(now + 2 * self.cfg.msl);
    }

    /// Start or stop the persist timer: running while the peer's window is
    /// closed and data waits with nothing in flight.
    pub(super) fn update_persist(&mut self, now: Instant) {
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

    pub(super) fn persist_interval(&self) -> Duration {
        let base = self.rtt.rto();
        let d = base.checked_mul(1u32 << self.persist_backoff.min(16)).unwrap_or(self.cfg.max_rto);
        d.min(self.cfg.max_rto)
    }

    pub(super) fn unsent(&self) -> usize {
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
}
