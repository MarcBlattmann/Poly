//! Socket table bookkeeping: slots, ports and bind conflicts.

use super::*;

impl Stack {
    pub(super) fn free_slots(&self) -> usize {
        let used = self.sockets.iter().filter(|e| e.is_some()).count();
        self.cfg.max_sockets.saturating_sub(used)
    }

    pub(super) fn insert(&mut self, mut e: Entry) -> Result<usize, Errno> {
        if self.free_slots() == 0 {
            return Err(Errno::ENOBUFS);
        }
        let idx = match self.sockets.iter().position(|s| s.is_none()) {
            Some(i) => i,
            None => {
                self.sockets.push(None);
                self.generations.push(0);
                self.sockets.len() - 1
            }
        };
        e.generation = self.generations[idx];
        self.sockets[idx] = Some(e);
        Ok(idx)
    }

    pub(super) fn free(&mut self, idx: usize) {
        self.sockets[idx] = None;
        self.generations[idx] = self.generations[idx].wrapping_add(1);
    }

    pub(super) fn handle_of(&self, idx: usize) -> SocketHandle {
        SocketHandle { index: idx as u32, generation: self.generations[idx] }
    }

    pub(super) fn entry(&mut self, h: SocketHandle) -> Result<&mut Entry, Errno> {
        match self.sockets.get_mut(h.index as usize) {
            Some(Some(e)) if e.user && e.generation == h.generation => Ok(e),
            _ => Err(Errno::EBADF),
        }
    }

    pub(super) fn entry_ref(&self, h: SocketHandle) -> Result<&Entry, Errno> {
        match self.sockets.get(h.index as usize) {
            Some(Some(e)) if e.user && e.generation == h.generation => Ok(e),
            _ => Err(Errno::EBADF),
        }
    }

    /// Bound ports of `protocol` (TCP, UDP or ICMP sockets) and the socket
    /// holding them, for conflict checks.
    pub(super) fn bound(&self) -> impl Iterator<Item = (usize, Protocol, SocketAddr, bool, Option<TcpState>)> + '_ {
        self.sockets.iter().enumerate().filter_map(|(i, e)| {
            let e = e.as_ref()?;
            match &e.sock {
                Sock::Tcp(t) if t.local.port() != 0 => Some((i, Protocol::Tcp, t.local, e.reuse_addr, Some(t.state))),
                Sock::Dgram(s) => s.local.map(|l| (i, s.proto, l, e.reuse_addr, None)),
                _ => None,
            }
        })
    }

    pub(super) fn port_in_use(&self, p: Protocol, port: u16) -> bool {
        self.bound().any(|(_, bp, a, _, _)| bp == p && a.port() == port)
    }

    /// Would binding `addr` conflict with an existing socket?
    pub(super) fn bind_conflict(&self, me: usize, p: Protocol, addr: SocketAddr, reuse: bool) -> bool {
        let reuse_port = |i: usize| self.sockets.get(i).and_then(Option::as_ref).is_some_and(|e| e.reuse_port);
        self.bound().any(|(i, bp, a, other_reuse, state)| {
            if i == me || bp != p || a.port() != addr.port() || a.is_ipv4() != addr.is_ipv4() {
                return false;
            }
            let overlap = a.ip() == addr.ip() || a.ip().is_unspecified() || addr.ip().is_unspecified();
            if !overlap {
                return false;
            }
            // SO_REUSEPORT on both: the port is shared.
            if reuse_port(me) && reuse_port(i) {
                return false;
            }
            // SO_REUSEADDR: TCP may share a port with connections that are
            // not listening (TIME-WAIT among them); UDP if both ask.
            let ok = match state {
                Some(s) => reuse && s != TcpState::Listen && (other_reuse || s == TcpState::TimeWait),
                None => reuse && other_reuse,
            };
            !ok
        })
    }

    pub(super) fn ephemeral_port(&mut self, p: Protocol) -> Result<u16, Errno> {
        let (lo, hi) = self.cfg.ephemeral_ports;
        let (lo, hi) = (u32::from(lo.min(hi)), u32::from(lo.max(hi)));
        let n = hi - lo + 1;
        for i in 0..n {
            let port = (lo + (self.next_port.wrapping_add(i)) % n) as u16;
            if !self.port_in_use(p, port) {
                self.next_port = self.next_port.wrapping_add(i + 1);
                return Ok(port);
            }
        }
        Err(Errno::EADDRNOTAVAIL)
    }
}
