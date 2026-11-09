//! User calls: send, receive, shutdown, close, abort, keep-alive and the readiness and statistics accessors.

use super::*;

impl Tcb {
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
    pub(super) fn maybe_window_update(&mut self) {
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

    pub(super) fn queue_fin(&mut self) {
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

    pub(super) fn become_closed(&mut self) {
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
    pub(super) fn fail(&mut self, e: Errno) {
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
}
