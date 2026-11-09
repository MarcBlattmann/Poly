//! Socket options, addresses and TCP info.

use super::*;

impl Stack {
    /// Bytes a read would return now (`FIONREAD` / `SIOCINQ`): queued
    /// stream bytes for TCP (`EINVAL` on a listening socket, tcp(7)), the
    /// size of the next datagram or 0 for datagram sockets (udp(7)).
    pub fn bytes_readable(&self, h: SocketHandle) -> Result<usize, Errno> {
        match &self.entry_ref(h)?.sock {
            Sock::Tcp(t) if t.state == TcpState::Listen => Err(Errno::EINVAL),
            Sock::Tcp(t) => Ok(t.recv_queued()),
            Sock::Dgram(s) => Ok(s.rx.front().map_or(0, |d| d.data.len())),
        }
    }

    /// Bytes queued for sending and not yet acknowledged (TCP) or not yet
    /// sent (datagrams): `SIOCOUTQ`.
    pub fn send_queued(&self, h: SocketHandle) -> Result<usize, Errno> {
        match &self.entry_ref(h)?.sock {
            Sock::Tcp(t) if t.state == TcpState::Listen => Err(Errno::EINVAL),
            Sock::Tcp(t) => Ok(t.send_queued()),
            Sock::Dgram(s) => Ok(s.tx_bytes),
        }
    }

    /// The send and receive buffer sizes, as `(send, recv)`.
    pub fn buffer_sizes(&self, h: SocketHandle) -> Result<(usize, usize), Errno> {
        Ok(match &self.entry_ref(h)?.sock {
            Sock::Tcp(t) => (t.cfg.send_buffer, t.cfg.recv_buffer),
            Sock::Dgram(s) => (s.tx_cap, s.rx_cap),
        })
    }

    /// The stack's default TTL / hop limit.
    pub fn default_ttl(&self) -> u8 {
        self.cfg.ttl
    }

    /// `IP_TTL` / `IPV6_UNICAST_HOPS` (`None`: the stack's default) and
    /// `IP_TOS` of a socket's packets.
    pub fn set_ip_options(&mut self, h: SocketHandle, ttl: Option<u8>, tos: u8) -> Result<(), Errno> {
        match &mut self.entry(h)?.sock {
            Sock::Tcp(t) => (t.ttl, t.tos) = (ttl, tos),
            Sock::Dgram(s) => (s.ttl, s.tos) = (ttl, tos),
        }
        Ok(())
    }

    /// The socket's TTL (`None`: the default) and TOS.
    pub fn ip_options(&self, h: SocketHandle) -> Result<(Option<u8>, u8), Errno> {
        Ok(match &self.entry_ref(h)?.sock {
            Sock::Tcp(t) => (t.ttl, t.tos),
            Sock::Dgram(s) => (s.ttl, s.tos),
        })
    }

    /// `TCP_KEEPIDLE`, `TCP_KEEPINTVL`, `TCP_KEEPCNT`: the keepalive
    /// timing of one TCP socket (`None` leaves a value alone). A running
    /// keepalive timer starts over with the new idle time.
    pub fn set_keepalive_params(
        &mut self,
        h: SocketHandle,
        idle: Option<Duration>,
        interval: Option<Duration>,
        probes: Option<u32>,
    ) -> Result<(), Errno> {
        let now = self.now;
        match &mut self.entry(h)?.sock {
            Sock::Tcp(t) => {
                if let Some(v) = idle {
                    t.cfg.keepalive_idle = v;
                }
                if let Some(v) = interval {
                    t.cfg.keepalive_interval = v;
                }
                if let Some(v) = probes {
                    t.cfg.keepalive_probes = v;
                }
                let on = t.keepalive;
                t.set_keepalive(on, now);
                Ok(())
            }
            Sock::Dgram(_) => Err(Errno::ENOPROTOOPT),
        }
    }

    /// The keepalive timing: `(idle, interval, probes)`.
    pub fn keepalive_params(&self, h: SocketHandle) -> Result<(Duration, Duration, u32), Errno> {
        match &self.entry_ref(h)?.sock {
            Sock::Tcp(t) => Ok((t.cfg.keepalive_idle, t.cfg.keepalive_interval, t.cfg.keepalive_probes)),
            Sock::Dgram(_) => Err(Errno::ENOPROTOOPT),
        }
    }

    /// The boolean options: `SO_REUSEADDR`, `SO_KEEPALIVE`, `TCP_NODELAY`,
    /// `SO_BROADCAST`.
    pub fn flags(&self, h: SocketHandle) -> Result<SocketFlags, Errno> {
        let e = self.entry_ref(h)?;
        Ok(match &e.sock {
            Sock::Tcp(t) => {
                SocketFlags { reuse_addr: e.reuse_addr, keepalive: t.keepalive, nodelay: t.nodelay, broadcast: false }
            }
            Sock::Dgram(s) => {
                SocketFlags { reuse_addr: e.reuse_addr, keepalive: false, nodelay: false, broadcast: s.broadcast }
            }
        })
    }

    /// Close with a reset (`SO_LINGER` with a zero timeout, socket(7) and
    /// RFC 9293 3.10.5 ABORT): a connection sends RST and is gone; other
    /// sockets close as [`close`](Self::close) does.
    pub fn abort(&mut self, h: SocketHandle) -> Result<(), Errno> {
        let idx = h.index as usize;
        let e = self.entry(h)?;
        match &mut e.sock {
            Sock::Tcp(t) if t.listen.is_none() && t.state != TcpState::Closed => {
                e.user = false;
                t.orphan = true;
                t.abort();
                self.settle(idx);
                Ok(())
            }
            _ => self.close(h),
        }
    }

    /// Take the pending error (`getsockopt(SO_ERROR)`).
    pub fn take_error(&mut self, h: SocketHandle) -> Result<Option<Errno>, Errno> {
        Ok(match &mut self.entry(h)?.sock {
            Sock::Tcp(t) => t.error.take(),
            Sock::Dgram(s) => s.error.take(),
        })
    }

    /// The local address (`getsockname(2)`).
    pub fn local_addr(&self, h: SocketHandle) -> Result<SocketAddr, Errno> {
        let e = self.entry_ref(h)?;
        Ok(match &e.sock {
            Sock::Tcp(t) if t.local.port() != 0 => t.local,
            Sock::Tcp(_) => e.domain.any(0),
            Sock::Dgram(s) => s.local.unwrap_or(e.domain.any(0)),
        })
    }

    /// The peer's address (`getpeername(2)`).
    pub fn peer_addr(&self, h: SocketHandle) -> Result<SocketAddr, Errno> {
        match &self.entry_ref(h)?.sock {
            Sock::Tcp(t) if !matches!(t.state, TcpState::Closed | TcpState::Listen) => Ok(t.remote),
            Sock::Dgram(s) => s.remote.ok_or(Errno::ENOTCONN),
            Sock::Tcp(_) => Err(Errno::ENOTCONN),
        }
    }

    /// `TCP_NODELAY`: turn Nagle's algorithm off (true) or on.
    pub fn set_nodelay(&mut self, h: SocketHandle, on: bool) -> Result<(), Errno> {
        match &mut self.entry(h)?.sock {
            Sock::Tcp(t) => {
                t.nodelay = on;
                Ok(())
            }
            Sock::Dgram(_) => Err(Errno::ENOPROTOOPT),
        }
    }

    /// `SO_KEEPALIVE`.
    pub fn set_keepalive(&mut self, h: SocketHandle, on: bool) -> Result<(), Errno> {
        let now = self.now;
        if let Sock::Tcp(t) = &mut self.entry(h)?.sock {
            t.set_keepalive(on, now);
        }
        Ok(())
    }

    /// `SO_REUSEADDR`.
    pub fn set_reuse_addr(&mut self, h: SocketHandle, on: bool) -> Result<(), Errno> {
        self.entry(h)?.reuse_addr = on;
        Ok(())
    }

    /// `SO_REUSEPORT`: share the address and port with other sockets that
    /// set it.
    pub fn set_reuse_port(&mut self, h: SocketHandle, on: bool) -> Result<(), Errno> {
        self.entry(h)?.reuse_port = on;
        Ok(())
    }

    /// `SO_BROADCAST`: allow sending to broadcast addresses.
    pub fn set_broadcast(&mut self, h: SocketHandle, on: bool) -> Result<(), Errno> {
        if let Sock::Dgram(s) = &mut self.entry(h)?.sock {
            s.broadcast = on;
        }
        Ok(())
    }

    /// `SO_SNDBUF` / `SO_RCVBUF`. TCP sockets take them only before they
    /// connect or listen (`EISCONN` after).
    pub fn set_buffer_sizes(&mut self, h: SocketHandle, send: Option<usize>, recv: Option<usize>) -> Result<(), Errno> {
        match &mut self.entry(h)?.sock {
            Sock::Tcp(t) => {
                if t.set_buffers(send, recv) {
                    Ok(())
                } else {
                    Err(Errno::EISCONN)
                }
            }
            Sock::Dgram(s) => {
                if let Some(v) = send {
                    s.tx_cap = v;
                }
                if let Some(v) = recv {
                    s.rx_cap = v;
                }
                Ok(())
            }
        }
    }

    /// The TCP state of a socket.
    pub fn tcp_state(&self, h: SocketHandle) -> Result<TcpState, Errno> {
        match &self.entry_ref(h)?.sock {
            Sock::Tcp(t) => Ok(t.state),
            Sock::Dgram(_) => Err(Errno::ENOPROTOOPT),
        }
    }

    /// Connection details (`TCP_INFO`-like).
    pub fn tcp_info(&self, h: SocketHandle) -> Result<TcpInfo, Errno> {
        match &self.entry_ref(h)?.sock {
            Sock::Tcp(t) => Ok(TcpInfo {
                state: t.state,
                stats: t.stats,
                srtt: t.srtt(),
                cwnd: t.cwnd(),
                send_mss: t.send_mss(),
                window_scales: t.window_scales(),
                timestamps: t.timestamps_enabled(),
                send_queued: t.send_queued(),
                recv_queued: t.recv_queued(),
            }),
            Sock::Dgram(_) => Err(Errno::ENOPROTOOPT),
        }
    }

    /// Sockets in use, including unaccepted and orphaned connections.
    pub fn socket_count(&self) -> usize {
        self.sockets.iter().filter(|e| e.is_some()).count()
    }

    /// The MAC address the neighbour cache has for `ip`, if resolved.
    pub fn neighbor(&self, ip: IpAddr) -> Option<MacAddr> {
        match ip {
            IpAddr::V4(a) => self.arp.peek(a),
            IpAddr::V6(a) => self.ndp.peek(a),
        }
    }
}
