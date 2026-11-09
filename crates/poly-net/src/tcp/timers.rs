//! Timers: retransmit, delayed ACK, persist, keep-alive and TIME-WAIT.

use super::*;

impl Tcb {
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

    pub(super) fn on_retransmit_timeout(&mut self, now: Instant) {
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
}
