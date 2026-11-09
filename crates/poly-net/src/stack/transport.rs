//! TCP and UDP input.

use super::*;

impl Stack {
    pub(super) fn tcp_input<D: NetDevice>(&mut self, dev: &mut D, src: IpAddr, dst: IpAddr, seg: &[u8], unicast: bool) {
        if !unicast {
            // RFC 9293 3.10.7.2: segments to broadcast or multicast
            // addresses are dropped.
            self.stats.rx_ignored += 1;
            return;
        }
        if !tcp::verify(seg, pseudo(src, dst, proto::TCP, seg.len())) {
            self.stats.rx_checksum += 1;
            return;
        }
        let (h, payload) = match tcp::parse(seg) {
            Ok(x) => x,
            Err(e) => {
                self.count_error(e);
                return;
            }
        };
        let now = self.now;
        if let Some(idx) = self.find_tcp_conn(dst, h.dst_port, src, h.src_port) {
            let entry = self.sockets[idx].as_mut().expect("found");
            let Sock::Tcp(tcb) = &mut entry.sock else { return };
            let reopen = tcb.state == TcpState::TimeWait
                && h.has(tcp::SYN)
                && !h.has(tcp::ACK)
                && !entry.user
                && tcb.time_wait_accepts(Seq(h.seq));
            if reopen {
                // RFC 1122 4.2.2.13: a new SYN may end TIME-WAIT early.
                self.free(idx);
            } else {
                let rst = tcb.input(now, &h, payload);
                let wants = tcb.wants_to_send();
                if let Some(r) = rst {
                    self.stats.tcp_rst_out += 1;
                    let _ = self.send_tcp(dev, dst, src, &r, &[]);
                }
                // Answer right away (ACKs for every second segment, data the
                // ACK let through), not only at the end of the poll.
                if wants && let Some(mut entry) = self.sockets[idx].take() {
                    if let Sock::Tcp(tcb) = &mut entry.sock {
                        let mut blocked = false;
                        self.tcp_output(dev, tcb, &mut blocked);
                    }
                    self.sockets[idx] = Some(entry);
                }
                self.settle(idx);
                return;
            }
        }
        if let Some(l) = self.find_listener(dst, h.dst_port) {
            self.listener_input(dev, l, src, dst, &h);
            return;
        }
        if let Some(r) = crate::tcp::rst_for_closed(&h, payload.len()) {
            self.stats.tcp_rst_out += 1;
            let _ = self.send_tcp(dev, dst, src, &r, &[]);
        }
    }

    pub(super) fn listener_input<D: NetDevice>(
        &mut self,
        dev: &mut D,
        lidx: usize,
        src: IpAddr,
        dst: IpAddr,
        h: &tcp::TcpHeader,
    ) {
        if h.has(tcp::RST) {
            return;
        }
        if h.has(tcp::ACK) {
            self.stats.tcp_rst_out += 1;
            let _ = self.send_tcp(dev, dst, src, &crate::tcp::rst_for_ack(h), &[]);
            return;
        }
        if !h.has(tcp::SYN) {
            return;
        }
        let Some(Entry { sock: Sock::Tcp(l), domain, .. }) = self.sockets[lidx].as_ref() else { return };
        let (cfg, nodelay, keepalive, domain) = (l.cfg, l.nodelay, l.keepalive, *domain);
        let (ttl, tos) = (l.ttl, l.tos);
        // The backlog bounds connections waiting for accept, established or
        // not; half-open ones are bounded separately as well.
        let full = l
            .listen
            .as_ref()
            .is_none_or(|q| q.ready.len() + q.half_open >= q.backlog || q.half_open >= cfg.syn_backlog);
        if full || self.free_slots() == 0 {
            self.stats.syn_dropped += 1;
            return;
        }
        let local = SocketAddr::new(dst, h.dst_port);
        let remote = SocketAddr::new(src, h.src_port);
        let iss = self.isn(local, remote);
        let mut child = Tcb::from_syn(cfg, nodelay, local, remote, h, iss, self.mss_limit(src), self.now);
        child.parent = Some(lidx);
        child.keepalive = keepalive;
        (child.ttl, child.tos) = (ttl, tos);
        let Ok(_) = self.insert(Entry {
            generation: 0,
            user: false,
            domain,
            reuse_addr: false,
            reuse_port: false,
            sock: Sock::Tcp(Box::new(child)),
            seen: None,
        }) else {
            return;
        };
        if let Some(Entry { sock: Sock::Tcp(l), .. }) = self.sockets[lidx].as_mut()
            && let Some(q) = l.listen.as_mut()
        {
            q.half_open += 1;
        }
    }

    /// After anything happened to a TCP socket: move connections that just
    /// got established to their listener's queue, free finished ones nobody
    /// holds.
    pub(super) fn settle(&mut self, idx: usize) {
        let Some(entry) = self.sockets[idx].as_mut() else { return };
        let user = entry.user;
        let Sock::Tcp(tcb) = &mut entry.sock else { return };
        let state = tcb.state;
        let parent = tcb.parent;
        let queued = tcb.accept_queued;
        if let Some(p) = parent {
            let established = !matches!(state, TcpState::SynReceived | TcpState::Closed);
            if established && !queued {
                tcb.accept_queued = true;
                if let Some(q) = self.listen_queue(p) {
                    q.half_open = q.half_open.saturating_sub(1);
                    q.ready.push_back(idx);
                }
            }
        }
        if state == TcpState::Closed && !user {
            if let Some(p) = parent
                && let Some(q) = self.listen_queue(p)
            {
                if queued {
                    q.ready.retain(|&i| i != idx);
                } else {
                    q.half_open = q.half_open.saturating_sub(1);
                }
            }
            self.free(idx);
        }
    }

    pub(super) fn listen_queue(&mut self, idx: usize) -> Option<&mut crate::tcp::ListenQueue> {
        match self.sockets.get_mut(idx)?.as_mut()?.sock {
            Sock::Tcp(ref mut l) => l.listen.as_mut(),
            Sock::Dgram(_) => None,
        }
    }

    pub(super) fn find_tcp_conn(&self, local: IpAddr, lport: u16, remote: IpAddr, rport: u16) -> Option<usize> {
        let (l, r) = (SocketAddr::new(local, lport), SocketAddr::new(remote, rport));
        self.sockets.iter().position(|e| match e {
            Some(Entry { sock: Sock::Tcp(t), .. }) => {
                !matches!(t.state, TcpState::Closed | TcpState::Listen) && t.local == l && t.remote == r
            }
            _ => false,
        })
    }

    pub(super) fn find_listener(&self, dst: IpAddr, port: u16) -> Option<usize> {
        let mut wildcard = None;
        for (i, e) in self.sockets.iter().enumerate() {
            if let Some(Entry { sock: Sock::Tcp(t), .. }) = e
                && t.state == TcpState::Listen
                && t.local.port() == port
                && t.local.is_ipv4() == dst.is_ipv4()
            {
                if t.local.ip() == dst {
                    return Some(i);
                }
                if t.local.ip().is_unspecified() {
                    wildcard = Some(i);
                }
            }
        }
        wildcard
    }

    /// Initial sequence number per RFC 6528: a 4-microsecond clock plus a
    /// keyed hash of the connection's addresses.
    pub(super) fn isn(&self, local: SocketAddr, remote: SocketAddr) -> Seq {
        #[cfg(test)]
        if let Some(v) = self.isn_override {
            return Seq(v);
        }
        let mut h = SipHasher::new(self.cfg.secret, self.cfg.secret.rotate_left(32) ^ 0x6e65_7473);
        hash_addr(&mut h, local);
        hash_addr(&mut h, remote);
        let m = (self.now.millis() as u32).wrapping_mul(250);
        Seq(m.wrapping_add(h.finish() as u32))
    }

    // ---- UDP input ---------------------------------------------------

    /// Deliver a UDP datagram. `Some(false)` means no socket wanted it
    /// (send port unreachable); `None` means it was invalid.
    pub(super) fn udp_input<D: NetDevice>(
        &mut self,
        _dev: &mut D,
        src: IpAddr,
        dst: IpAddr,
        seg: &[u8],
    ) -> Option<bool> {
        let (h, data) = match udp::parse(seg) {
            Ok(x) => x,
            Err(e) => {
                self.count_error(e);
                return None;
            }
        };
        let ps = pseudo(src, dst, proto::UDP, h.len);
        if !udp::verify(seg, &h, ps, src.is_ipv4()) {
            self.stats.rx_checksum += 1;
            return None;
        }
        let from = SocketAddr::new(src, h.src_port);
        let mut best: Option<(usize, u8)> = None;
        for (i, e) in self.sockets.iter().enumerate() {
            let Some(Entry { sock: Sock::Dgram(s), .. }) = e else { continue };
            if s.proto != Protocol::Udp {
                continue;
            }
            let Some(l) = s.local else { continue };
            if l.port() != h.dst_port || l.is_ipv4() != dst.is_ipv4() {
                continue;
            }
            let addr_match = l.ip() == dst;
            if !addr_match && !l.ip().is_unspecified() {
                continue;
            }
            if let Some(r) = s.remote
                && r != from
            {
                continue;
            }
            let score = u8::from(addr_match) * 2 + u8::from(s.remote.is_some());
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((i, score));
            }
        }
        let Some((i, _)) = best else {
            self.stats.udp_no_port += 1;
            return Some(false);
        };
        if let Some(Entry { sock: Sock::Dgram(s), .. }) = self.sockets[i].as_mut() {
            s.deliver(from, data);
        }
        Some(true)
    }
}
