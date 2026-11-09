//! The BSD-style socket API: create, bind, connect, send, receive, close.

use super::*;

impl Stack {
    /// Create a socket (`socket(2)`). `protocol` `None` picks TCP for
    /// streams and UDP for datagrams.
    pub fn socket(&mut self, domain: Domain, ty: SockType, protocol: Option<Protocol>) -> Result<SocketHandle, Errno> {
        let p = match (ty, protocol) {
            (SockType::Stream, None | Some(Protocol::Tcp)) => Protocol::Tcp,
            (SockType::Datagram, None | Some(Protocol::Udp)) => Protocol::Udp,
            (SockType::Datagram, Some(Protocol::Icmp)) => Protocol::Icmp,
            _ => return Err(Errno::EPROTONOSUPPORT),
        };
        let sock = match p {
            Protocol::Tcp => Sock::Tcp(Box::new(Tcb::new(self.cfg.tcp, domain.any(0), self.now))),
            _ => Sock::Dgram(DgramSocket::new(p, self.cfg.udp_recv_buffer, self.cfg.udp_send_buffer)),
        };
        let idx = self.insert(Entry {
            generation: 0,
            user: true,
            domain,
            reuse_addr: false,
            reuse_port: false,
            sock,
            seen: None,
        })?;
        Ok(self.handle_of(idx))
    }

    pub(super) fn protocol_of(e: &Entry) -> Protocol {
        match &e.sock {
            Sock::Tcp(_) => Protocol::Tcp,
            Sock::Dgram(s) => s.proto,
        }
    }

    /// Bind to a local address (`bind(2)`); port 0 picks an ephemeral port.
    pub fn bind(&mut self, h: SocketHandle, addr: SocketAddr) -> Result<(), Errno> {
        let e = self.entry_ref(h)?;
        if Domain::of(&addr) != e.domain {
            return Err(Errno::EAFNOSUPPORT);
        }
        let p = Self::protocol_of(e);
        let reuse = e.reuse_addr;
        let already = match &e.sock {
            Sock::Tcp(t) => t.local.port() != 0 || t.state != TcpState::Closed,
            Sock::Dgram(s) => s.local.is_some(),
        };
        if already {
            return Err(Errno::EINVAL);
        }
        if !self.is_local_addr(addr.ip()) {
            return Err(Errno::EADDRNOTAVAIL);
        }
        let port = if addr.port() == 0 {
            self.ephemeral_port(p)?
        } else if self.bind_conflict(h.index as usize, p, addr, reuse) {
            return Err(Errno::EADDRINUSE);
        } else {
            addr.port()
        };
        let local = SocketAddr::new(addr.ip(), port);
        match &mut self.entry(h)?.sock {
            Sock::Tcp(t) => t.local = local,
            Sock::Dgram(s) => s.local = Some(local),
        }
        Ok(())
    }

    /// Listen for connections (`listen(2)`).
    pub fn listen(&mut self, h: SocketHandle, backlog: usize) -> Result<(), Errno> {
        let e = self.entry_ref(h)?;
        let Sock::Tcp(t) = &e.sock else { return Err(Errno::EOPNOTSUPP) };
        if !matches!(t.state, TcpState::Closed | TcpState::Listen) || t.was_connected {
            return Err(Errno::EINVAL);
        }
        if t.local.port() == 0 {
            let domain = e.domain;
            self.bind(h, domain.any(0))?;
        } else if t.state == TcpState::Closed {
            let (local, reuse) = (t.local, e.reuse_addr);
            let me = h.index as usize;
            let shared = |i: usize| {
                [me, i].iter().all(|&j| self.sockets.get(j).and_then(Option::as_ref).is_some_and(|e| e.reuse_port))
            };
            let listener_conflict = self.bound().any(|(i, bp, a, _, s)| {
                i != me
                    && !shared(i)
                    && bp == Protocol::Tcp
                    && s == Some(TcpState::Listen)
                    && a.port() == local.port()
                    && a.is_ipv4() == local.is_ipv4()
                    && (a.ip() == local.ip() || a.ip().is_unspecified() || local.ip().is_unspecified())
            });
            if listener_conflict || (!reuse && self.bind_conflict(h.index as usize, Protocol::Tcp, local, false)) {
                return Err(Errno::EADDRINUSE);
            }
        }
        let Sock::Tcp(t) = &mut self.entry(h)?.sock else { return Err(Errno::EOPNOTSUPP) };
        t.listen(backlog.clamp(1, 4096));
        Ok(())
    }

    /// Take an established connection (`accept(2)`): its handle and the
    /// peer's address. `EAGAIN` if none is waiting.
    pub fn accept(&mut self, h: SocketHandle) -> Result<(SocketHandle, SocketAddr), Errno> {
        let lidx = h.index as usize;
        loop {
            let Sock::Tcp(t) = &mut self.entry(h)?.sock else { return Err(Errno::EOPNOTSUPP) };
            if t.state != TcpState::Listen {
                return Err(Errno::EINVAL);
            }
            let Some(q) = t.listen.as_mut() else { return Err(Errno::EINVAL) };
            let Some(idx) = q.ready.pop_front() else { return Err(Errno::EAGAIN) };
            if let Some(Some(Entry { sock: Sock::Tcp(c), user, .. })) = self.sockets.get_mut(idx)
                && c.parent == Some(lidx)
            {
                c.parent = None;
                *user = true;
                let peer = c.remote;
                return Ok((self.handle_of(idx), peer));
            }
        }
    }

    /// Connect (`connect(2)`). TCP returns `EINPROGRESS`; the socket turns
    /// writable when connected, or reports the failure through
    /// [`take_error`](Self::take_error). Datagram sockets just set their
    /// default destination and filter what they receive.
    pub fn connect(&mut self, h: SocketHandle, addr: SocketAddr) -> Result<(), Errno> {
        let e = self.entry_ref(h)?;
        if Domain::of(&addr) != e.domain {
            return Err(Errno::EAFNOSUPPORT);
        }
        let mut remote = addr;
        if remote.ip().is_unspecified() {
            // Connecting to the unspecified address means this host.
            remote.set_ip(match e.domain {
                Domain::Inet => IpAddr::V4(Ipv4Addr::LOCALHOST),
                Domain::Inet6 => IpAddr::V6(Ipv6Addr::LOCALHOST),
            });
        }
        let p = Self::protocol_of(e);
        match &e.sock {
            Sock::Tcp(t) => {
                match t.state {
                    TcpState::Closed => {}
                    TcpState::SynSent | TcpState::SynReceived => return Err(Errno::EALREADY),
                    TcpState::Listen => return Err(Errno::EINVAL),
                    _ => return Err(Errno::EISCONN),
                }
                if remote.ip().is_multicast() || matches!(remote.ip(), IpAddr::V4(v) if v.is_broadcast()) {
                    return Err(Errno::ENETUNREACH);
                }
            }
            Sock::Dgram(_) => {}
        }
        let src = self.source_for(remote.ip())?;
        self.hop(remote.ip())?;
        let bound = match &self.entry_ref(h)?.sock {
            Sock::Tcp(t) => (t.local.port() != 0).then_some(t.local),
            Sock::Dgram(s) => s.local,
        };
        let port = match bound {
            Some(b) => b.port(),
            None => self.ephemeral_port(p)?,
        };
        let local_ip = match bound {
            Some(b) if !b.ip().is_unspecified() => b.ip(),
            _ => src,
        };
        let local = SocketAddr::new(local_ip, port);
        if p == Protocol::Tcp && self.find_tcp_conn(local.ip(), local.port(), remote.ip(), remote.port()).is_some() {
            return Err(Errno::EADDRNOTAVAIL);
        }
        let iss = self.isn(local, remote);
        let mss = self.mss_limit(remote.ip());
        let now = self.now;
        match &mut self.entry(h)?.sock {
            Sock::Tcp(t) => {
                if t.was_connected {
                    // A failed connection may be retried: start afresh,
                    // keeping the options.
                    let mut fresh = Tcb::new(t.cfg, local, now);
                    fresh.nodelay = t.nodelay;
                    fresh.keepalive = t.keepalive;
                    **t = fresh;
                }
                t.connect(local, remote, iss, mss, now);
                Err(Errno::EINPROGRESS)
            }
            Sock::Dgram(s) => {
                s.local = Some(local);
                s.remote = Some(remote);
                s.error = None;
                Ok(())
            }
        }
    }

    /// Dissolve a datagram socket's association (`connect` with `AF_UNSPEC`).
    pub fn disconnect(&mut self, h: SocketHandle) -> Result<(), Errno> {
        match &mut self.entry(h)?.sock {
            Sock::Dgram(s) => {
                s.remote = None;
                Ok(())
            }
            Sock::Tcp(_) => Err(Errno::EOPNOTSUPP),
        }
    }

    /// Send on a connected socket (`send(2)`); returns the bytes taken.
    pub fn send(&mut self, h: SocketHandle, data: &[u8]) -> Result<usize, Errno> {
        self.send_to(h, data, None)
    }

    /// Send a datagram to `to`, or on a connected socket (`sendto(2)`).
    /// TCP ignores `to`. ICMP sockets take a whole echo request (type,
    /// code, checksum, identifier, sequence, data); the identifier and
    /// checksum are filled in.
    pub fn send_to(&mut self, h: SocketHandle, data: &[u8], to: Option<SocketAddr>) -> Result<usize, Errno> {
        let mtu = self.mtu;
        let e = self.entry(h)?;
        let domain = e.domain;
        let s = match &mut e.sock {
            Sock::Tcp(t) => return t.send(data),
            Sock::Dgram(s) => s,
        };
        if s.wr_shut {
            return Err(Errno::EPIPE);
        }
        if let Some(err) = s.error.take() {
            return Err(err);
        }
        let Some(dst) = to.or(s.remote) else { return Err(Errno::EDESTADDRREQ) };
        if Domain::of(&dst) != domain {
            return Err(Errno::EAFNOSUPPORT);
        }
        let proto = s.proto;
        let broadcast_ok = s.broadcast;
        let (overhead, max) = match proto {
            Protocol::Udp => (udp::HEADER_LEN, 65535 - udp::HEADER_LEN),
            _ => (0, 65535),
        };
        let ip = if dst.is_ipv4() { 20 } else { 40 };
        let mtu = mtu.max(self.path_mtu(dst.ip()));
        if data.len() > max || data.len() + overhead + ip > mtu {
            // No fragmentation of outgoing datagrams (see the crate docs).
            return Err(Errno::EMSGSIZE);
        }
        if proto == Protocol::Icmp {
            let want = if dst.is_ipv4() { icmpv4::ECHO_REQUEST } else { icmpv6::ECHO_REQUEST };
            if data.len() < 8 || data[0] != want || data[1] != 0 {
                return Err(Errno::EINVAL);
            }
        }
        if let IpAddr::V4(v) = dst.ip()
            && !broadcast_ok
            && (v.is_broadcast() || self.cfg.ipv4.is_some_and(|c| c.prefix_len < 31 && c.broadcast() == v))
        {
            return Err(Errno::EACCES);
        }
        self.hop(dst.ip())?;
        let needs_port = match &self.entry_ref(h)?.sock {
            Sock::Dgram(s) => s.local.is_none(),
            Sock::Tcp(_) => false,
        };
        let port = if needs_port { Some(self.ephemeral_port(proto)?) } else { None };
        let Sock::Dgram(s) = &mut self.entry(h)?.sock else { return Err(Errno::EBADF) };
        if let Some(port) = port {
            s.local = Some(domain.any(port));
        }
        if s.tx_bytes + data.len() > s.tx_cap {
            return Err(Errno::EAGAIN);
        }
        s.tx_bytes += data.len();
        s.tx.push_back(Datagram { addr: dst, data: data.to_vec() });
        Ok(data.len())
    }

    /// Receive (`recv(2)`). TCP returns `Ok(0)` at end of stream.
    pub fn recv(&mut self, h: SocketHandle, buf: &mut [u8]) -> Result<usize, Errno> {
        self.recv_from(h, buf, RecvFlags::default()).map(|i| i.len)
    }

    /// Receive with the sender's address (`recvfrom(2)`).
    pub fn recv_from(&mut self, h: SocketHandle, buf: &mut [u8], flags: RecvFlags) -> Result<RecvInfo, Errno> {
        match &mut self.entry(h)?.sock {
            Sock::Tcp(t) => {
                let n = t.recv(buf, flags.peek)?;
                Ok(RecvInfo { len: n, full_len: n, from: t.remote })
            }
            Sock::Dgram(s) => s.recv(buf, flags),
        }
    }

    /// Shut down one or both directions (`shutdown(2)`).
    pub fn shutdown(&mut self, h: SocketHandle, how: Shutdown) -> Result<(), Errno> {
        let idx = h.index as usize;
        match &mut self.entry(h)?.sock {
            // A listener shut down for reading stops listening: accept is
            // EINVAL from then on, and connections nobody accepted are reset
            // (shutdown's and accept4's spec sheets).
            Sock::Tcp(t) if t.state == TcpState::Listen => {
                if matches!(how, Shutdown::Read | Shutdown::Both) {
                    t.listen = None;
                    t.state = TcpState::Closed;
                    self.reset_unaccepted(idx);
                }
                Ok(())
            }
            Sock::Tcp(t) => {
                if t.state == TcpState::Closed {
                    return Err(Errno::ENOTCONN);
                }
                if matches!(how, Shutdown::Read | Shutdown::Both) {
                    t.shutdown_read();
                }
                if matches!(how, Shutdown::Write | Shutdown::Both) {
                    t.shutdown_write();
                }
                Ok(())
            }
            Sock::Dgram(s) => {
                if s.remote.is_none() {
                    return Err(Errno::ENOTCONN);
                }
                if matches!(how, Shutdown::Read | Shutdown::Both) {
                    s.rd_shut = true;
                }
                if matches!(how, Shutdown::Write | Shutdown::Both) {
                    s.wr_shut = true;
                }
                Ok(())
            }
        }
    }

    /// Close the handle (`close(2)`). A TCP connection then finishes in the
    /// background (FIN, TIME-WAIT), or is reset if unread data remained.
    pub fn close(&mut self, h: SocketHandle) -> Result<(), Errno> {
        let now = self.now;
        let idx = h.index as usize;
        let e = self.entry(h)?;
        e.user = false;
        let (was_listening, done) = match &mut e.sock {
            Sock::Tcp(t) => (t.listen.take().is_some(), t.close(now)),
            Sock::Dgram(_) => (false, true),
        };
        if was_listening {
            self.reset_unaccepted(idx);
        }
        if done {
            self.free(idx);
        } else {
            self.settle(idx);
        }
        Ok(())
    }

    /// Reset the connections listener `idx` has that nobody accepted.
    pub(super) fn reset_unaccepted(&mut self, idx: usize) {
        for (i, c) in self.sockets.iter_mut().enumerate() {
            if let Some(Entry { sock: Sock::Tcp(c), .. }) = c
                && c.parent == Some(idx)
                && i != idx
            {
                c.parent = None;
                c.orphan = true;
                c.abort();
            }
        }
    }

    /// Readiness flags of a socket.
    pub fn readiness(&self, h: SocketHandle) -> Result<Readiness, Errno> {
        Ok(Self::readiness_of(self.entry_ref(h)?))
    }

    pub(super) fn readiness_of(e: &Entry) -> Readiness {
        match &e.sock {
            Sock::Tcp(t) => {
                let mut r = Readiness::default();
                if t.readable() {
                    r |= Readiness::READABLE;
                }
                if t.writable() {
                    r |= Readiness::WRITABLE;
                }
                if t.error.is_some() {
                    r |= Readiness::ERROR;
                }
                if t.hangup() {
                    r |= Readiness::HANGUP;
                }
                if t.fin_received || t.rd_shut {
                    r |= Readiness::READ_HANGUP;
                }
                r
            }
            Sock::Dgram(s) => s.readiness(),
        }
    }

    /// What [`take_changed`](Self::take_changed) compares: the readiness
    /// and how much input is queued (bytes, datagrams, connections), so
    /// that more data arriving on a socket that was readable already
    /// counts as a change too (edge-triggered waiters need it).
    pub(super) fn signature(e: &Entry) -> (Readiness, usize) {
        let queued = match &e.sock {
            Sock::Tcp(t) => t.recv_queued() + t.listen.as_ref().map_or(0, |l| l.ready.len()),
            Sock::Dgram(s) => s.rx_bytes + s.rx.len(),
        };
        (Self::readiness_of(e), queued)
    }

    /// Append to `out` every open socket whose readiness or queued input
    /// changed since the previous call (or that is new since then), so a
    /// kernel can wake only the waiters of those sockets. Call it after
    /// [`poll`](Self::poll) and after socket calls; it costs one look at
    /// every socket.
    pub fn take_changed(&mut self, out: &mut Vec<SocketHandle>) {
        for idx in 0..self.sockets.len() {
            let Some(e) = self.sockets[idx].as_mut() else { continue };
            if !e.user {
                continue;
            }
            let sig = Self::signature(e);
            if e.seen != Some(sig) {
                e.seen = Some(sig);
                out.push(self.handle_of(idx));
            }
        }
    }
}
