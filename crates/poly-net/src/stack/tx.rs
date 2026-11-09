//! Send path: route choice, IP output, TCP and datagram transmit.

use super::*;

impl Stack {
    /// Where packets to `dst` go.
    pub(super) fn hop(&self, dst: IpAddr) -> Result<Hop, Errno> {
        match dst {
            IpAddr::V4(d) => {
                if d.is_loopback() {
                    return Ok(Hop::Loop);
                }
                let Some(c) = self.cfg.ipv4 else { return Err(Errno::ENETUNREACH) };
                if d == c.addr {
                    Ok(Hop::Loop)
                } else if d.is_broadcast() || (c.prefix_len < 31 && d == c.broadcast()) {
                    Ok(Hop::Mac(MacAddr::BROADCAST))
                } else if d.is_multicast() {
                    Ok(Hop::Mac(MacAddr::ipv4_multicast(d)))
                } else if d.is_unspecified() {
                    Err(Errno::ENETUNREACH)
                } else if c.contains(d) {
                    Ok(Hop::Neighbor4(d))
                } else if let Some(gw) = self.cfg.gateway {
                    Ok(Hop::Neighbor4(gw))
                } else {
                    Err(Errno::ENETUNREACH)
                }
            }
            IpAddr::V6(d) => {
                if d.is_loopback() || self.is_our_v6(d) {
                    return Ok(Hop::Loop);
                }
                if !self.cfg.ipv6 || d.is_unspecified() {
                    return Err(Errno::ENETUNREACH);
                }
                if d.is_multicast() {
                    Ok(Hop::Mac(MacAddr::ipv6_multicast(d)))
                } else if ipv6::is_link_local(d) || self.cfg.ipv6_global.is_some_and(|c| c.contains(d)) {
                    Ok(Hop::Neighbor6(d))
                } else if let Some(gw) = self.cfg.gateway6 {
                    Ok(Hop::Neighbor6(gw))
                } else {
                    Err(Errno::ENETUNREACH)
                }
            }
        }
    }

    /// True unless packets to `dst` would only pile up behind an unanswered
    /// neighbour request.
    pub(super) fn path_ready(&self, dst: IpAddr) -> bool {
        match self.hop(dst) {
            Ok(Hop::Neighbor4(nh)) => self.arp.peek(nh).is_some() || !self.arp.has_pending(nh),
            Ok(Hop::Neighbor6(nh)) => self.ndp.peek(nh).is_some() || !self.ndp.has_pending(nh),
            _ => true,
        }
    }

    /// Source address for packets to `dst`.
    pub(super) fn source_for(&self, dst: IpAddr) -> Result<IpAddr, Errno> {
        match dst {
            IpAddr::V4(d) if d.is_loopback() => Ok(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            IpAddr::V4(_) => self.cfg.ipv4.map(|c| IpAddr::V4(c.addr)).ok_or(Errno::ENETUNREACH),
            IpAddr::V6(d) if d.is_loopback() => Ok(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            IpAddr::V6(d) => {
                if !self.cfg.ipv6 {
                    return Err(Errno::ENETUNREACH);
                }
                Ok(IpAddr::V6(self.ipv6_source_for(d)))
            }
        }
    }

    pub(super) fn ipv6_source_for(&self, dst: Ipv6Addr) -> Ipv6Addr {
        match self.cfg.ipv6_global {
            Some(g) if !ipv6::is_link_local(dst) && !dst.is_multicast() => g.addr,
            _ => self.ipv6_ll,
        }
    }

    pub(super) fn our_v6_addrs(&self) -> impl Iterator<Item = Ipv6Addr> + '_ {
        let ll = self.cfg.ipv6.then_some(self.ipv6_ll);
        ll.into_iter().chain(self.cfg.ipv6_global.filter(|_| self.cfg.ipv6).map(|c| c.addr))
    }

    pub(super) fn is_our_v6(&self, a: Ipv6Addr) -> bool {
        self.our_v6_addrs().any(|x| x == a)
    }

    pub(super) fn is_local_addr(&self, a: IpAddr) -> bool {
        match a {
            IpAddr::V4(v) => v.is_unspecified() || v.is_loopback() || self.cfg.ipv4.is_some_and(|c| c.addr == v),
            IpAddr::V6(v) => v.is_unspecified() || v.is_loopback() || self.is_our_v6(v),
        }
    }

    /// The largest IP packet towards `dst`: the device's MTU, or for a
    /// destination on this host [`ipv4::MAX_PACKET_LEN`] (loopback carries
    /// whole 64 KiB datagrams, as Linux's `lo` does: sendto's spec sheet,
    /// 65507 bytes of UDP).
    pub(super) fn path_mtu(&self, dst: IpAddr) -> usize {
        if self.is_local_addr(dst) { ipv4::MAX_PACKET_LEN } else { self.mtu }
    }

    /// [`output_ip`](Self::output_ip) for a packet to this host: built on
    /// its own (it may be larger than a device frame) and queued for the
    /// next [`poll`](Self::poll).
    pub(super) fn output_loop(
        &mut self,
        src: IpAddr,
        dst: IpAddr,
        protocol: u8,
        payload: &[u8],
        dont_fragment: bool,
    ) -> Result<(), Errno> {
        let packet = match (src, dst) {
            (IpAddr::V4(s), IpAddr::V4(d)) => {
                if 20 + payload.len() > ipv4::MAX_PACKET_LEN {
                    return Err(Errno::EMSGSIZE);
                }
                let mut h = ipv4::Ipv4Header::new(s, d, protocol, self.pkt_ttl.unwrap_or(self.cfg.ttl), payload.len());
                h.tos = self.pkt_tos;
                h.dont_fragment = dont_fragment;
                h.ident = self.ip_ident;
                self.ip_ident = self.ip_ident.wrapping_add(1);
                let mut p = vec![0u8; 20 + payload.len()];
                ipv4::emit(&mut p, &h);
                p[20..].copy_from_slice(payload);
                p
            }
            (IpAddr::V6(s), IpAddr::V6(d)) => {
                if payload.len() > ipv4::MAX_PACKET_LEN {
                    return Err(Errno::EMSGSIZE);
                }
                let h = ipv6::Ipv6Header {
                    traffic_class: 0,
                    flow_label: 0,
                    payload_len: payload.len(),
                    next_header: protocol,
                    hop_limit: self.pkt_ttl.unwrap_or(self.cfg.ttl),
                    src: s,
                    dst: d,
                };
                let mut p = vec![0u8; 40 + payload.len()];
                ipv6::emit(&mut p, &h);
                p[40..].copy_from_slice(payload);
                p
            }
            _ => return Err(Errno::EAFNOSUPPORT),
        };
        if self.loopq.len() < LOOP_QUEUE {
            self.loopq.push_back(packet);
        } else {
            self.stats.loop_dropped += 1;
        }
        Ok(())
    }

    /// Largest TCP segment payload towards `dst`.
    pub(super) fn mss_limit(&self, dst: IpAddr) -> usize {
        let ip = if dst.is_ipv4() { 20 } else { 40 };
        self.mtu - ip - 20
    }

    /// Build and send an IP packet carrying `payload`.
    pub(super) fn output_ip<D: NetDevice>(
        &mut self,
        dev: &mut D,
        src: IpAddr,
        dst: IpAddr,
        protocol: u8,
        payload: &[u8],
        dont_fragment: bool,
    ) -> Result<(), Errno> {
        let hop = self.hop(dst)?;
        if matches!(hop, Hop::Loop) {
            return self.output_loop(src, dst, protocol, payload, dont_fragment);
        }
        let mut frame = core::mem::take(&mut self.frame_buf);
        let o = ethernet::HEADER_LEN;
        let (ip_len, ethertype) = match (src, dst) {
            (IpAddr::V4(s), IpAddr::V4(d)) => {
                if 20 + payload.len() > self.mtu {
                    self.frame_buf = frame;
                    return Err(Errno::EMSGSIZE);
                }
                let mut h = ipv4::Ipv4Header::new(s, d, protocol, self.pkt_ttl.unwrap_or(self.cfg.ttl), payload.len());
                h.tos = self.pkt_tos;
                h.dont_fragment = dont_fragment;
                h.ident = self.ip_ident;
                self.ip_ident = self.ip_ident.wrapping_add(1);
                ipv4::emit(&mut frame[o..], &h);
                frame[o + 20..o + 20 + payload.len()].copy_from_slice(payload);
                (20 + payload.len(), ETHERTYPE_IPV4)
            }
            (IpAddr::V6(s), IpAddr::V6(d)) => {
                if 40 + payload.len() > self.mtu {
                    self.frame_buf = frame;
                    return Err(Errno::EMSGSIZE);
                }
                let h = ipv6::Ipv6Header {
                    traffic_class: 0,
                    flow_label: 0,
                    payload_len: payload.len(),
                    next_header: protocol,
                    hop_limit: self.pkt_ttl.unwrap_or(self.cfg.ttl),
                    src: s,
                    dst: d,
                };
                ipv6::emit(&mut frame[o..], &h);
                frame[o + 40..o + 40 + payload.len()].copy_from_slice(payload);
                (40 + payload.len(), ETHERTYPE_IPV6)
            }
            _ => {
                self.frame_buf = frame;
                return Err(Errno::EAFNOSUPPORT);
            }
        };
        let packet = &frame[o..o + ip_len];
        match hop {
            Hop::Loop => {
                if self.loopq.len() < LOOP_QUEUE {
                    self.loopq.push_back(packet.to_vec());
                } else {
                    self.stats.loop_dropped += 1;
                }
            }
            Hop::Mac(mac) => self.transmit(dev, &mut frame, mac, ethertype, ip_len),
            Hop::Neighbor4(nh) => {
                let l = self.arp.lookup(nh, self.now);
                if l.request {
                    self.send_arp_request(dev, nh);
                }
                match l.mac {
                    Some(mac) => self.transmit(dev, &mut frame, mac, ethertype, ip_len),
                    None => {
                        if self.arp.enqueue(nh, Pending { ethertype, packet: packet.to_vec() }, self.now) {
                            self.send_arp_request(dev, nh);
                        }
                    }
                }
            }
            Hop::Neighbor6(nh) => {
                let l = self.ndp.lookup(nh, self.now);
                if l.request {
                    self.send_neighbor_solicit(dev, nh);
                }
                match l.mac {
                    Some(mac) => self.transmit(dev, &mut frame, mac, ethertype, ip_len),
                    None => {
                        if self.ndp.enqueue(nh, Pending { ethertype, packet: packet.to_vec() }, self.now) {
                            self.send_neighbor_solicit(dev, nh);
                        }
                    }
                }
            }
        }
        self.frame_buf = frame;
        Ok(())
    }

    /// Put an Ethernet header in front of the packet at
    /// `frame[14..14 + ip_len]`, pad, and hand it to the device.
    pub(super) fn transmit<D: NetDevice>(
        &mut self,
        dev: &mut D,
        frame: &mut [u8],
        dst: MacAddr,
        ethertype: u16,
        ip_len: usize,
    ) {
        ethernet::write_header(frame, dst, self.mac, ethertype);
        let mut len = ethernet::HEADER_LEN + ip_len;
        if len < ethernet::MIN_FRAME_LEN {
            frame[len..ethernet::MIN_FRAME_LEN].fill(0);
            len = ethernet::MIN_FRAME_LEN;
        }
        match dev.transmit(&frame[..len]) {
            Ok(()) => self.stats.tx_frames += 1,
            Err(TxError::Busy | TxError::TooLong) => self.stats.tx_dropped += 1,
        }
    }

    /// Send a packet (ARP, or an IP packet that waited) in a frame to `dst`.
    pub(super) fn send_frame<D: NetDevice>(&mut self, dev: &mut D, dst: MacAddr, ethertype: u16, packet: &[u8]) {
        let mut frame = core::mem::take(&mut self.aux_buf);
        let n = packet.len().min(frame.len() - ethernet::HEADER_LEN);
        frame[ethernet::HEADER_LEN..ethernet::HEADER_LEN + n].copy_from_slice(&packet[..n]);
        self.transmit(dev, &mut frame, dst, ethertype, n);
        self.aux_buf = frame;
    }

    pub(super) fn send_tcp<D: NetDevice>(
        &mut self,
        dev: &mut D,
        src: IpAddr,
        dst: IpAddr,
        h: &tcp::TcpHeader,
        payload: &[u8],
    ) -> Result<(), Errno> {
        let mut b = core::mem::take(&mut self.tx_buf);
        let hl = h.header_len();
        let n = payload.len().min(b.len() - hl);
        b[hl..hl + n].copy_from_slice(&payload[..n]);
        let ps = pseudo(src, dst, proto::TCP, hl + n);
        let len = tcp::emit(&mut b, h, n, ps);
        let r = self.output_ip(dev, src, dst, proto::TCP, &b[..len], true);
        self.tx_buf = b;
        r
    }

    pub(super) fn tcp_output<D: NetDevice>(&mut self, dev: &mut D, tcb: &mut Tcb, blocked: &mut bool) {
        tcb.on_timers(self.now);
        (self.pkt_ttl, self.pkt_tos) = (tcb.ttl, tcb.tos);
        self.tcp_output_segments(dev, tcb, blocked);
        (self.pkt_ttl, self.pkt_tos) = (None, 0);
    }

    pub(super) fn tcp_output_segments<D: NetDevice>(&mut self, dev: &mut D, tcb: &mut Tcb, blocked: &mut bool) {
        for _ in 0..TCP_BUDGET {
            if !tcb.wants_to_send() || !self.path_ready(tcb.remote.ip()) {
                break;
            }
            if !dev.can_transmit() {
                *blocked = true;
                break;
            }
            let mut seg = core::mem::take(&mut self.seg_buf);
            let out = tcb.dispatch(self.now, &mut seg);
            if let Some((h, n)) = out {
                if h.flags & tcp::RST != 0 {
                    self.stats.tcp_rst_out += 1;
                }
                if let Err(e) = self.send_tcp(dev, tcb.local.ip(), tcb.remote.ip(), &h, &seg[..n]) {
                    tcb.unreachable(e);
                }
            }
            self.seg_buf = seg;
        }
    }

    pub(super) fn dgram_output<D: NetDevice>(&mut self, dev: &mut D, s: &mut DgramSocket, blocked: &mut bool) {
        (self.pkt_ttl, self.pkt_tos) = (s.ttl, s.tos);
        self.dgram_output_queue(dev, s, blocked);
        (self.pkt_ttl, self.pkt_tos) = (None, 0);
    }

    pub(super) fn dgram_output_queue<D: NetDevice>(&mut self, dev: &mut D, s: &mut DgramSocket, blocked: &mut bool) {
        while let Some(d) = s.tx.front() {
            if !self.path_ready(d.addr.ip()) {
                break;
            }
            if !dev.can_transmit() {
                *blocked = true;
                break;
            }
            let Some(d) = s.tx.pop_front() else { break };
            s.tx_bytes -= d.data.len();
            let local = s.local.unwrap_or(Domain::of(&d.addr).any(0));
            let src = if local.ip().is_unspecified() {
                match self.source_for(d.addr.ip()) {
                    Ok(a) => a,
                    Err(e) => {
                        s.error = Some(e);
                        continue;
                    }
                }
            } else {
                local.ip()
            };
            let r = match s.proto {
                Protocol::Udp => {
                    // A datagram larger than a device frame (to this host,
                    // `path_mtu`) gets a buffer of its own.
                    let need = udp::HEADER_LEN + d.data.len();
                    let own = self.tx_buf.len() < need;
                    let mut b = if own { vec![0u8; need] } else { core::mem::take(&mut self.tx_buf) };
                    let n = d.data.len().min(b.len() - udp::HEADER_LEN);
                    b[udp::HEADER_LEN..udp::HEADER_LEN + n].copy_from_slice(&d.data[..n]);
                    let ps = pseudo(src, d.addr.ip(), proto::UDP, udp::HEADER_LEN + n);
                    udp::emit(&mut b, local.port(), d.addr.port(), n, ps);
                    let r = self.output_ip(dev, src, d.addr.ip(), proto::UDP, &b[..udp::HEADER_LEN + n], false);
                    if !own {
                        self.tx_buf = b;
                    }
                    r
                }
                _ => {
                    // Echo request: the identifier is the socket's port.
                    let mut b = core::mem::take(&mut self.tx_buf);
                    let n = d.data.len().min(b.len());
                    b[..n].copy_from_slice(&d.data[..n]);
                    b[4..6].copy_from_slice(&local.port().to_be_bytes());
                    let p = match (src, d.addr.ip()) {
                        (IpAddr::V6(s6), IpAddr::V6(d6)) => {
                            icmpv6::fix_checksum(&mut b[..n], s6, d6);
                            proto::ICMPV6
                        }
                        _ => {
                            icmpv4::fix_checksum(&mut b[..n]);
                            proto::ICMP
                        }
                    };
                    let r = self.output_ip(dev, src, d.addr.ip(), p, &b[..n], false);
                    self.tx_buf = b;
                    r
                }
            };
            if let Err(e) = r
                && s.remote.is_some()
            {
                s.error = Some(e);
            }
        }
    }
}
