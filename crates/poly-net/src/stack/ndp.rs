//! Neighbour discovery (ARP/NDP) and the timers that run it and reassembly.

use super::*;

impl Stack {
    pub(super) fn ndp_solicit_input<D: NetDevice>(
        &mut self,
        dev: &mut D,
        ih: &ipv6::Ipv6Header,
        m: &icmpv6::Message<'_>,
    ) {
        // RFC 4861 7.1.1 validity checks.
        if ih.hop_limit != 255 || m.code != 0 {
            return;
        }
        let Ok(n) = icmpv6::parse_neighbor(m.data, icmpv6::OPT_SOURCE_LLADDR) else { return };
        if n.target.is_multicast() || !self.is_our_v6(n.target) || ih.src.is_unspecified() {
            // Duplicate address detection probes (unspecified source) are
            // not answered: this stack does not do DAD.
            return;
        }
        let mac = match n.lladdr {
            Some(mac) if mac.is_unicast() => {
                let released = self.ndp.update(ih.src, mac, self.now, true);
                for pkt in released {
                    self.send_frame(dev, mac, pkt.ethertype, &pkt.packet);
                }
                mac
            }
            _ => match self.ndp.peek(ih.src) {
                Some(mac) => mac,
                None => return,
            },
        };
        let mut b = [0u8; 32];
        let len = icmpv6::write_neighbor(&mut b, n.target, icmpv6::OPT_TARGET_LLADDR, self.mac);
        icmpv6::emit(
            &mut b,
            len,
            icmpv6::NEIGHBOR_ADVERT,
            0,
            icmpv6::NA_SOLICITED | icmpv6::NA_OVERRIDE,
            n.target,
            ih.src,
        );
        self.send_ipv6_direct(dev, mac, n.target, ih.src, &b[..len]);
        self.stats.ndp_adverts_out += 1;
    }

    pub(super) fn ndp_advert_input<D: NetDevice>(
        &mut self,
        dev: &mut D,
        ih: &ipv6::Ipv6Header,
        m: &icmpv6::Message<'_>,
    ) {
        if ih.hop_limit != 255 || m.code != 0 {
            return;
        }
        let Ok(n) = icmpv6::parse_neighbor(m.data, icmpv6::OPT_TARGET_LLADDR) else { return };
        if n.target.is_multicast() || (ih.dst.is_multicast() && m.rest & icmpv6::NA_SOLICITED != 0) {
            return;
        }
        if let Some(mac) = n.lladdr.filter(|m| m.is_unicast()) {
            // RFC 4861 7.2.5: only entries that exist are updated.
            let released = self.ndp.update(n.target, mac, self.now, false);
            for pkt in released {
                self.send_frame(dev, mac, pkt.ethertype, &pkt.packet);
            }
        }
    }

    pub(super) fn send_ipv6_direct<D: NetDevice>(
        &mut self,
        dev: &mut D,
        mac: MacAddr,
        src: Ipv6Addr,
        dst: Ipv6Addr,
        body: &[u8],
    ) {
        let mut b = [0u8; 40 + 32];
        let h = ipv6::Ipv6Header {
            traffic_class: 0,
            flow_label: 0,
            payload_len: body.len(),
            next_header: proto::ICMPV6,
            hop_limit: 255,
            src,
            dst,
        };
        ipv6::emit(&mut b, &h);
        b[40..40 + body.len()].copy_from_slice(body);
        self.send_frame(dev, mac, ETHERTYPE_IPV6, &b[..40 + body.len()]);
    }

    pub(super) fn send_arp_request<D: NetDevice>(&mut self, dev: &mut D, target: Ipv4Addr) {
        let Some(cidr) = self.cfg.ipv4 else { return };
        let p = arp::ArpPacket {
            op: arp::OP_REQUEST,
            sender_mac: self.mac,
            sender_ip: cidr.addr,
            target_mac: MacAddr::ZERO,
            target_ip: target,
        };
        let mut b = [0u8; arp::LEN];
        p.emit(&mut b);
        self.send_frame(dev, MacAddr::BROADCAST, ETHERTYPE_ARP, &b);
        self.stats.arp_requests_out += 1;
    }

    pub(super) fn send_neighbor_solicit<D: NetDevice>(&mut self, dev: &mut D, target: Ipv6Addr) {
        let src = self.ipv6_source_for(target);
        let dst = ipv6::solicited_node(target);
        let mut b = [0u8; 32];
        let len = icmpv6::write_neighbor(&mut b, target, icmpv6::OPT_SOURCE_LLADDR, self.mac);
        icmpv6::emit(&mut b, len, icmpv6::NEIGHBOR_SOLICIT, 0, 0, src, dst);
        self.send_ipv6_direct(dev, MacAddr::ipv6_multicast(dst), src, dst, &b[..len]);
        self.stats.ndp_solicits_out += 1;
    }

    pub(super) fn run_neighbor_timers<D: NetDevice>(&mut self, dev: &mut D) {
        let now = self.now;
        let mut requests4 = Vec::new();
        let mut failed: Vec<Pending> = Vec::new();
        self.arp.poll(now, |a| requests4.push(a), |_, p| failed.extend(p));
        for a in requests4 {
            self.send_arp_request(dev, a);
        }
        let mut requests6 = Vec::new();
        self.ndp.poll(now, |a| requests6.push(a), |_, p| failed.extend(p));
        for a in requests6 {
            self.send_neighbor_solicit(dev, a);
        }
        for p in failed {
            self.neighbor_failed(&p.packet);
        }
    }

    /// A locally sent packet could not be delivered: its next hop did not
    /// answer. Tell the sender (`EHOSTUNREACH`).
    pub(super) fn neighbor_failed(&mut self, packet: &[u8]) {
        let err = IcmpError::Unreachable { errno: Errno::EHOSTUNREACH, hard: true };
        match packet.first().map(|b| b >> 4) {
            Some(4) => {
                if let Ok((h, payload)) = ipv4::parse(packet) {
                    self.transport_error(h.protocol, IpAddr::V4(h.src), IpAddr::V4(h.dst), payload, err);
                }
            }
            Some(6) => {
                if let Ok((h, payload)) = ipv6::parse(packet)
                    && let Ok((next, off)) = ipv6::skip_extensions(h.next_header, payload)
                {
                    self.transport_error(next, IpAddr::V6(h.src), IpAddr::V6(h.dst), &payload[off..], err);
                }
            }
            _ => {}
        }
    }

    pub(super) fn run_reassembly_timers<D: NetDevice>(&mut self, dev: &mut D) {
        let mut expired = Vec::new();
        self.frag.poll(self.now, |e| expired.push(e));
        for e in expired {
            // RFC 792 / RFC 1122 3.3.2: Time Exceeded, code 1, if the first
            // fragment arrived.
            let mut quote = [0u8; 28];
            let mut h = e.header;
            h.header_len = 20;
            ipv4::emit(&mut quote, &h);
            quote[20..20 + e.data_len].copy_from_slice(&e.data[..e.data_len]);
            let n = 20 + e.data_len;
            self.icmp4_error(dev, icmpv4::TIME_EXCEEDED, icmpv4::time_exceeded::REASSEMBLY, 0, &h, &quote[..n]);
        }
    }
}
