//! Receive path: frame, ARP and IP input.

use super::*;

impl Stack {
    pub(super) fn process_frame<D: NetDevice>(&mut self, dev: &mut D, buf: &[u8]) {
        let f = match ethernet::parse(buf) {
            Ok(f) => f,
            Err(_) => {
                self.stats.rx_malformed += 1;
                return;
            }
        };
        let ours = f.dst == self.mac || f.dst.is_broadcast() || self.accepts_multicast(f.dst);
        if !ours || f.src == self.mac || !f.src.is_unicast() {
            self.stats.rx_ignored += 1;
            return;
        }
        match f.ethertype {
            ETHERTYPE_ARP => self.process_arp(dev, f.payload),
            ETHERTYPE_IPV4 => self.process_ip(dev, f.payload, false),
            ETHERTYPE_IPV6 if self.cfg.ipv6 => self.process_ip(dev, f.payload, false),
            _ => self.stats.rx_ignored += 1,
        }
    }

    pub(super) fn accepts_multicast(&self, mac: MacAddr) -> bool {
        if !mac.is_multicast() {
            return false;
        }
        if mac == MacAddr::ipv4_multicast(Ipv4Addr::new(224, 0, 0, 1)) {
            return true;
        }
        self.cfg.ipv6
            && (mac == MacAddr::ipv6_multicast(ipv6::ALL_NODES)
                || self.our_v6_addrs().any(|a| mac == MacAddr::ipv6_multicast(ipv6::solicited_node(a))))
    }

    pub(super) fn process_arp<D: NetDevice>(&mut self, dev: &mut D, buf: &[u8]) {
        let Ok(p) = arp::ArpPacket::parse(buf) else {
            self.stats.rx_malformed += 1;
            return;
        };
        let Some(cidr) = self.cfg.ipv4 else { return };
        if !p.sender_mac.is_unicast() || p.sender_mac == self.mac {
            return;
        }
        let for_us = p.target_ip == cidr.addr;
        // RFC 826: merge the sender into the cache if present, add it if
        // the packet is for us. Senders without an address (RFC 5227
        // probes) and off-subnet senders are not cached.
        let sender_ok = !p.sender_ip.is_unspecified() && ipv4::is_valid_source(p.sender_ip) && p.sender_ip != cidr.addr;
        if sender_ok {
            let released = self.arp.update(p.sender_ip, p.sender_mac, self.now, for_us && cidr.contains(p.sender_ip));
            for pkt in released {
                self.send_frame(dev, p.sender_mac, pkt.ethertype, &pkt.packet);
            }
        }
        if p.op == arp::OP_REQUEST && for_us {
            let reply = arp::ArpPacket {
                op: arp::OP_REPLY,
                sender_mac: self.mac,
                sender_ip: cidr.addr,
                target_mac: p.sender_mac,
                target_ip: p.sender_ip,
            };
            let mut b = [0u8; arp::LEN];
            reply.emit(&mut b);
            self.send_frame(dev, p.sender_mac, ETHERTYPE_ARP, &b);
            self.stats.arp_replies_out += 1;
        }
    }

    pub(super) fn process_ip<D: NetDevice>(&mut self, dev: &mut D, packet: &[u8], from_loop: bool) {
        match packet.first().map(|b| b >> 4) {
            Some(4) => self.process_ipv4(dev, packet, from_loop),
            Some(6) if self.cfg.ipv6 || from_loop => self.process_ipv6(dev, packet, from_loop),
            _ => self.stats.rx_malformed += 1,
        }
    }

    pub(super) fn process_ipv4<D: NetDevice>(&mut self, dev: &mut D, packet: &[u8], from_loop: bool) {
        let (h, payload) = match ipv4::parse(packet) {
            Ok(x) => x,
            Err(e) => {
                self.count_error(e);
                return;
            }
        };
        if !ipv4::is_valid_source(h.src) {
            self.stats.rx_ignored += 1;
            return;
        }
        let cidr = self.cfg.ipv4;
        let to_our_addr = cidr.is_some_and(|c| c.addr == h.dst);
        let broadcast = h.dst.is_broadcast() || cidr.is_some_and(|c| c.prefix_len < 31 && c.broadcast() == h.dst);
        let multicast = h.dst == Ipv4Addr::new(224, 0, 0, 1);
        let for_us = if from_loop {
            h.dst.is_loopback() || to_our_addr
        } else {
            // Loopback addresses never come from the wire (RFC 1122 3.2.1.3).
            !h.src.is_loopback() && (to_our_addr || broadcast || multicast)
        };
        if !for_us {
            self.stats.rx_ignored += 1;
            return;
        }
        let unicast = !broadcast && !multicast;
        if h.is_fragment() {
            if let Some((full, data)) = self.frag.insert(self.now, &h, payload) {
                self.deliver_v4(dev, &full, &data, unicast);
            }
            return;
        }
        self.deliver_v4(dev, &h, payload, unicast);
    }

    pub(super) fn deliver_v4<D: NetDevice>(
        &mut self,
        dev: &mut D,
        h: &ipv4::Ipv4Header,
        payload: &[u8],
        unicast: bool,
    ) {
        let (src, dst) = (IpAddr::V4(h.src), IpAddr::V4(h.dst));
        match h.protocol {
            proto::TCP => self.tcp_input(dev, src, dst, payload, unicast),
            proto::UDP => {
                if self.udp_input(dev, src, dst, payload) == Some(false) && unicast {
                    let quote = quote_v4(h, payload);
                    self.icmp4_error(dev, icmpv4::DEST_UNREACHABLE, icmpv4::unreachable::PORT, 0, h, &quote);
                }
            }
            proto::ICMP => self.icmpv4_input(dev, h, payload, unicast),
            _ => {
                self.stats.rx_ignored += 1;
                if unicast {
                    let quote = quote_v4(h, payload);
                    self.icmp4_error(dev, icmpv4::DEST_UNREACHABLE, icmpv4::unreachable::PROTOCOL, 0, h, &quote);
                }
            }
        }
    }

    pub(super) fn process_ipv6<D: NetDevice>(&mut self, dev: &mut D, packet: &[u8], from_loop: bool) {
        let (h, payload) = match ipv6::parse(packet) {
            Ok(x) => x,
            Err(e) => {
                self.count_error(e);
                return;
            }
        };
        if h.src.is_multicast() {
            self.stats.rx_ignored += 1;
            return;
        }
        let ours = self.is_our_v6(h.dst);
        let multicast = h.dst == ipv6::ALL_NODES || self.our_v6_addrs().any(|a| ipv6::solicited_node(a) == h.dst);
        let for_us = if from_loop {
            h.dst.is_loopback() || ours
        } else {
            !h.src.is_loopback() && !h.dst.is_loopback() && (ours || multicast)
        };
        if !for_us {
            self.stats.rx_ignored += 1;
            return;
        }
        let (next, off) = match ipv6::skip_extensions(h.next_header, payload) {
            Ok(x) => x,
            Err(e) => {
                self.count_error(e);
                return;
            }
        };
        let body = &payload[off..];
        let (src, dst) = (IpAddr::V6(h.src), IpAddr::V6(h.dst));
        match next {
            proto::TCP => self.tcp_input(dev, src, dst, body, !multicast),
            proto::UDP => {
                if self.udp_input(dev, src, dst, body) == Some(false) && !multicast {
                    let quote = quote_v6(&h, payload);
                    self.icmp6_error(dev, icmpv6::DEST_UNREACHABLE, icmpv6::unreachable::PORT, 0, &h, &quote);
                }
            }
            proto::ICMPV6 => self.icmpv6_input(dev, &h, body, multicast),
            _ => self.stats.rx_ignored += 1,
        }
    }

    pub(super) fn count_error(&mut self, e: WireError) {
        match e {
            WireError::Checksum => self.stats.rx_checksum += 1,
            WireError::Unsupported => self.stats.rx_ignored += 1,
            WireError::Truncated | WireError::Malformed => self.stats.rx_malformed += 1,
        }
    }
}
