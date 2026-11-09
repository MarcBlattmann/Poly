//! Segment output: deciding what to send and building the segment.

use super::*;

impl Tcb {
    pub(super) fn plan(&self) -> Option<Plan> {
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
    pub(super) fn advertise(&mut self) -> u16 {
        let free = self.rbuf.free();
        let cur = self.rcv_adv.since(self.rcv_nxt);
        let threshold = (self.rbuf.capacity() / 2).min(self.mss_limit).max(1);
        let wnd = if free >= cur + threshold || free == self.rbuf.capacity() { free } else { cur };
        let scaled = (wnd >> self.rcv_wscale).min(65535);
        let edge = self.rcv_nxt + (scaled << self.rcv_wscale);
        self.rcv_adv = self.rcv_adv.max(edge);
        scaled as u16
    }

    pub(super) fn timestamps(&self, now: Instant) -> Option<(u32, u32)> {
        self.ts_enabled.then(|| ((now.millis() as u32).wrapping_add(self.ts_offset), self.ts_recent))
    }

    pub(super) fn header(&self, seq: Seq, flags: u8, window: u16) -> TcpHeader {
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

    pub(super) fn syn_options(&self, now: Instant, syn_ack: bool) -> TcpOptions {
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
    pub(super) fn sent_seq_space(&mut self, now: Instant, seq: Seq, len: usize) {
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

    pub(super) fn ack_sent(&mut self) {
        self.ack_now = false;
        self.delack_at = None;
        self.unacked_bytes = 0;
        self.last_ack_sent = self.rcv_nxt;
    }
}
