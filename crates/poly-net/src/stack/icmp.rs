//! ICMP and ICMPv6: replies, errors and delivery to sockets.

use super::*;

impl Stack {
    pub(super) fn icmpv4_input<D: NetDevice>(
        &mut self,
        dev: &mut D,
        ih: &ipv4::Ipv4Header,
        payload: &[u8],
        unicast: bool,
    ) {
        let m = match icmpv4::parse(payload) {
            Ok(m) => m,
            Err(e) => {
                self.count_error(e);
                return;
            }
        };
        match m.ty {
            // RFC 1122 3.2.2.6 allows ignoring echo requests to broadcast
            // addresses; we do.
            // A reply that does not fit the MTU (to a reassembled request)
            // is not sent: there is no outgoing fragmentation.
            icmpv4::ECHO_REQUEST if unicast && self.cfg.reply_to_ping && payload.len() + 20 <= self.mtu => {
                let mut b = core::mem::take(&mut self.tx_buf);
                let len = payload.len();
                b[..len].copy_from_slice(payload);
                b[0] = icmpv4::ECHO_REPLY;
                icmpv4::fix_checksum(&mut b[..len]);
                let _ = self.output_ip(dev, IpAddr::V4(ih.dst), IpAddr::V4(ih.src), proto::ICMP, &b[..len], false);
                self.tx_buf = b;
            }
            icmpv4::ECHO_REPLY => {
                let from = sock_addr_v4(ih.src, 0);
                self.ping_deliver(Domain::Inet, m.ident(), from, payload);
            }
            icmpv4::DEST_UNREACHABLE | icmpv4::TIME_EXCEEDED | icmpv4::PARAMETER_PROBLEM => {
                let Ok((q, qdata)) = parse_quoted_v4(m.data) else { return };
                let err = match (m.ty, m.code) {
                    (icmpv4::DEST_UNREACHABLE, icmpv4::unreachable::FRAG_NEEDED) => {
                        let mtu = (m.rest & 0xffff) as usize;
                        if mtu < ipv4::MIN_MTU {
                            return;
                        }
                        IcmpError::PathMtu(mtu)
                    }
                    (icmpv4::DEST_UNREACHABLE, c) => {
                        let errno = match c {
                            icmpv4::unreachable::NET => Errno::ENETUNREACH,
                            icmpv4::unreachable::PROTOCOL => Errno::ENOPROTOOPT,
                            icmpv4::unreachable::PORT => Errno::ECONNREFUSED,
                            _ => Errno::EHOSTUNREACH,
                        };
                        let hard = matches!(c, icmpv4::unreachable::PROTOCOL | icmpv4::unreachable::PORT);
                        IcmpError::Unreachable { errno, hard }
                    }
                    _ => IcmpError::Unreachable { errno: Errno::EHOSTUNREACH, hard: false },
                };
                self.transport_error(q.protocol, IpAddr::V4(q.src), IpAddr::V4(q.dst), qdata, err);
            }
            _ => {}
        }
    }

    pub(super) fn icmpv6_input<D: NetDevice>(
        &mut self,
        dev: &mut D,
        ih: &ipv6::Ipv6Header,
        body: &[u8],
        multicast: bool,
    ) {
        let m = match icmpv6::parse(body, ih.src, ih.dst) {
            Ok(m) => m,
            Err(e) => {
                self.count_error(e);
                return;
            }
        };
        match m.ty {
            icmpv6::ECHO_REQUEST if self.cfg.reply_to_ping && body.len() + 40 <= self.mtu => {
                // RFC 4443 4.2: a reply to a multicast request comes from a
                // unicast address of ours.
                let src = if multicast { self.ipv6_source_for(ih.src) } else { ih.dst };
                let mut b = core::mem::take(&mut self.tx_buf);
                let len = body.len();
                b[..len].copy_from_slice(body);
                b[0] = icmpv6::ECHO_REPLY;
                icmpv6::fix_checksum(&mut b[..len], src, ih.src);
                let _ = self.output_ip(dev, IpAddr::V6(src), IpAddr::V6(ih.src), proto::ICMPV6, &b[..len], false);
                self.tx_buf = b;
            }
            icmpv6::ECHO_REPLY => {
                let from = SocketAddr::new(IpAddr::V6(ih.src), 0);
                self.ping_deliver(Domain::Inet6, (m.rest >> 16) as u16, from, body);
            }
            icmpv6::NEIGHBOR_SOLICIT => self.ndp_solicit_input(dev, ih, &m),
            icmpv6::NEIGHBOR_ADVERT => self.ndp_advert_input(dev, ih, &m),
            icmpv6::DEST_UNREACHABLE | icmpv6::PACKET_TOO_BIG | icmpv6::TIME_EXCEEDED | icmpv6::PARAMETER_PROBLEM => {
                let Ok((qh, qpayload)) = ipv6::parse_quoted(m.data) else { return };
                let Ok((next, off)) = ipv6::skip_extensions(qh.next_header, qpayload) else { return };
                let err = match (m.ty, m.code) {
                    (icmpv6::PACKET_TOO_BIG, _) => IcmpError::PathMtu((m.rest as usize).max(ipv6::MIN_MTU)),
                    (icmpv6::DEST_UNREACHABLE, c) => {
                        let errno = match c {
                            icmpv6::unreachable::NO_ROUTE => Errno::ENETUNREACH,
                            icmpv6::unreachable::ADMIN => Errno::EACCES,
                            icmpv6::unreachable::PORT => Errno::ECONNREFUSED,
                            _ => Errno::EHOSTUNREACH,
                        };
                        IcmpError::Unreachable { errno, hard: c == icmpv6::unreachable::PORT }
                    }
                    _ => IcmpError::Unreachable { errno: Errno::EHOSTUNREACH, hard: false },
                };
                self.transport_error(next, IpAddr::V6(qh.src), IpAddr::V6(qh.dst), &qpayload[off..], err);
            }
            _ => {}
        }
    }

    /// Spend one token of the ICMP error budget.
    pub(super) fn icmp_allowed(&mut self) -> bool {
        let elapsed = crate::time::ms(self.now.since(self.icmp_refill));
        self.icmp_refill = self.now;
        let cap = u64::from(self.cfg.icmp_burst) * 1000;
        let refill = elapsed.saturating_mul(u64::from(self.cfg.icmp_rate));
        self.icmp_tokens = self.icmp_tokens.saturating_add(refill).min(cap);
        if self.icmp_tokens >= 1000 {
            self.icmp_tokens -= 1000;
            true
        } else {
            self.stats.icmp_rate_limited += 1;
            false
        }
    }

    /// Send an ICMPv4 error about a packet whose header is `orig` (RFC 1122
    /// 3.2.2: never about an ICMP error, a non-initial fragment, or a
    /// packet not from a unicast source).
    pub(super) fn icmp4_error<D: NetDevice>(
        &mut self,
        dev: &mut D,
        ty: u8,
        code: u8,
        rest: u32,
        orig: &ipv4::Ipv4Header,
        quote: &[u8],
    ) {
        if orig.frag_offset != 0 || !ipv4::is_valid_source(orig.src) || orig.src.is_unspecified() {
            return;
        }
        if orig.protocol == proto::ICMP && quote.get(orig.header_len.min(20)).is_some_and(|&t| icmpv4::is_error(t)) {
            return;
        }
        let Some(cidr) = self.cfg.ipv4 else { return };
        if !self.icmp_allowed() {
            return;
        }
        let src = if orig.dst.is_loopback() || orig.dst == cidr.addr { orig.dst } else { cidr.addr };
        let n = quote.len().min(ICMP4_QUOTE).min(self.mtu - 28);
        let mut b = core::mem::take(&mut self.tx_buf);
        b[icmpv4::HEADER_LEN..icmpv4::HEADER_LEN + n].copy_from_slice(&quote[..n]);
        icmpv4::emit(&mut b, ty, code, rest, n);
        let _ = self.output_ip(dev, IpAddr::V4(src), IpAddr::V4(orig.src), proto::ICMP, &b[..8 + n], false);
        self.tx_buf = b;
        self.stats.icmp_out += 1;
    }

    pub(super) fn icmp6_error<D: NetDevice>(
        &mut self,
        dev: &mut D,
        ty: u8,
        code: u8,
        rest: u32,
        orig: &ipv6::Ipv6Header,
        quote: &[u8],
    ) {
        if orig.src.is_unspecified() || orig.src.is_multicast() || orig.dst.is_multicast() {
            return;
        }
        if !self.icmp_allowed() {
            return;
        }
        let src = if self.is_our_v6(orig.dst) { orig.dst } else { self.ipv6_source_for(orig.src) };
        let n = quote.len().min(ICMP6_QUOTE).min(self.mtu - 48);
        let mut b = core::mem::take(&mut self.tx_buf);
        b[8..8 + n].copy_from_slice(&quote[..n]);
        icmpv6::emit(&mut b, 8 + n, ty, code, rest, src, orig.src);
        let _ = self.output_ip(dev, IpAddr::V6(src), IpAddr::V6(orig.src), proto::ICMPV6, &b[..8 + n], false);
        self.tx_buf = b;
        self.stats.icmp_out += 1;
    }

    /// Hand an ICMP error (or a failed neighbour resolution) to the socket
    /// that sent the quoted packet. `transport` is its transport header.
    pub(super) fn transport_error(&mut self, protocol: u8, src: IpAddr, dst: IpAddr, transport: &[u8], err: IcmpError) {
        if transport.len() < 8 {
            return;
        }
        let sport = u16::from_be_bytes([transport[0], transport[1]]);
        let dport = u16::from_be_bytes([transport[2], transport[3]]);
        match protocol {
            proto::TCP => {
                let seq = Seq(u32::from_be_bytes([transport[4], transport[5], transport[6], transport[7]]));
                if let Some(idx) = self.find_tcp_conn(src, sport, dst, dport) {
                    if let Some(Sock::Tcp(t)) = self.sockets[idx].as_mut().map(|e| &mut e.sock) {
                        t.icmp_error(seq, err);
                    }
                    self.settle(idx);
                }
            }
            proto::UDP => {
                let IcmpError::Unreachable { errno, .. } = err else { return };
                let remote = SocketAddr::new(dst, dport);
                for e in self.sockets.iter_mut().flatten() {
                    if let Sock::Dgram(s) = &mut e.sock
                        && s.proto == Protocol::Udp
                        && s.local.is_some_and(|l| l.port() == sport)
                        && s.remote == Some(remote)
                    {
                        // Only connected sockets hear about errors.
                        s.error = Some(errno);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn ping_deliver(&mut self, domain: Domain, ident: u16, from: SocketAddr, msg: &[u8]) {
        for e in self.sockets.iter_mut().flatten() {
            if e.domain != domain {
                continue;
            }
            if let Sock::Dgram(s) = &mut e.sock
                && s.proto == Protocol::Icmp
                && s.local.is_some_and(|l| l.port() == ident)
                && s.remote.is_none_or(|r| r.ip() == from.ip())
            {
                s.deliver(from, msg);
                return;
            }
        }
    }
}
