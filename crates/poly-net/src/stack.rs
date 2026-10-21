//! The stack: one Ethernet interface (plus internal loopback), its
//! addresses, neighbour caches, fragment reassembly and all sockets.
//!
//! Driving it:
//! - [`Stack::poll`] receives frames from the device, runs timers and sends
//!   what is due; it returns the next deadline. Call it again at that time,
//!   when the device has received something, and after socket calls (they
//!   only queue work; nothing is sent from inside them).
//! - Socket calls never block: they return `EAGAIN` or `EINPROGRESS`, and
//!   [`Stack::readiness`] says when to try again.

use crate::device::{NetDevice, TxError};
use crate::errno::Errno;
use crate::neighbor::{NeighborCache, NeighborConfig, Pending};
use crate::reassembly::{Reassembler, ReassemblyConfig, ReassemblyStats};
use crate::siphash::SipHasher;
use crate::socket::{
    Datagram, DgramSocket, Domain, Protocol, Readiness, RecvFlags, RecvInfo, Shutdown, SockType, SocketHandle,
};
use crate::tcp::seq::Seq;
use crate::tcp::{IcmpError, Tcb, TcpConfig, TcpState, TcpStats};
use crate::time::{Instant, earliest};
use crate::wire::checksum;
use crate::wire::ethernet::{self, ETHERTYPE_ARP, ETHERTYPE_IPV4, ETHERTYPE_IPV6, MacAddr};
use crate::wire::{WireError, arp, icmpv4, icmpv6, ipv4, ipv6, proto, tcp, udp};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::time::Duration;

/// An IPv4 address with its prefix length (10.0.0.2/24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Cidr {
    pub addr: Ipv4Addr,
    pub prefix_len: u8,
}

impl Ipv4Cidr {
    pub fn new(addr: Ipv4Addr, prefix_len: u8) -> Ipv4Cidr {
        Ipv4Cidr { addr, prefix_len: prefix_len.min(32) }
    }

    fn mask(&self) -> u32 {
        if self.prefix_len == 0 { 0 } else { u32::MAX << (32 - u32::from(self.prefix_len)) }
    }

    /// True if `a` is on this subnet.
    pub fn contains(&self, a: Ipv4Addr) -> bool {
        (u32::from(a) ^ u32::from(self.addr)) & self.mask() == 0
    }

    /// The subnet's directed broadcast address.
    pub fn broadcast(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.addr) | !self.mask())
    }
}

/// An IPv6 address with its prefix length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv6Cidr {
    pub addr: Ipv6Addr,
    pub prefix_len: u8,
}

impl Ipv6Cidr {
    pub fn new(addr: Ipv6Addr, prefix_len: u8) -> Ipv6Cidr {
        Ipv6Cidr { addr, prefix_len: prefix_len.min(128) }
    }

    pub fn contains(&self, a: Ipv6Addr) -> bool {
        let mask = if self.prefix_len == 0 { 0 } else { u128::MAX << (128 - u32::from(self.prefix_len)) };
        (u128::from(a) ^ u128::from(self.addr)) & mask == 0
    }
}

/// Stack configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The interface's IPv4 address, if any.
    pub ipv4: Option<Ipv4Cidr>,
    /// Default IPv4 router.
    pub gateway: Option<Ipv4Addr>,
    /// Enable IPv6 with a link-local address derived from the MAC.
    pub ipv6: bool,
    /// An additional (global) IPv6 address.
    pub ipv6_global: Option<Ipv6Cidr>,
    /// Default IPv6 router.
    pub gateway6: Option<Ipv6Addr>,
    /// TTL / hop limit of packets sent.
    pub ttl: u8,
    pub tcp: TcpConfig,
    /// Receive buffer of datagram sockets, in payload bytes.
    pub udp_recv_buffer: usize,
    /// Send queue of datagram sockets, in payload bytes.
    pub udp_send_buffer: usize,
    pub neighbor: NeighborConfig,
    pub reassembly: ReassemblyConfig,
    /// Ephemeral port range, inclusive (Linux's default is 32768-60999).
    pub ephemeral_ports: (u16, u16),
    /// Sockets, including connections waiting to be accepted and closed
    /// connections finishing in the background.
    pub max_sockets: usize,
    /// Secret for initial sequence numbers and port choice. The kernel
    /// should pass real randomness here.
    pub secret: u64,
    /// ICMP error messages per second (RFC 1812 4.3.2.8 rate limiting).
    pub icmp_rate: u32,
    /// Burst of ICMP error messages allowed at once.
    pub icmp_burst: u32,
    /// Answer echo requests.
    pub reply_to_ping: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ipv4: None,
            gateway: None,
            ipv6: true,
            ipv6_global: None,
            gateway6: None,
            ttl: 64,
            tcp: TcpConfig::default(),
            udp_recv_buffer: 128 * 1024,
            udp_send_buffer: 64 * 1024,
            neighbor: NeighborConfig::default(),
            reassembly: ReassemblyConfig::default(),
            ephemeral_ports: (32768, 60999),
            max_sockets: 1024,
            secret: 0x0123_4567_89ab_cdef,
            icmp_rate: 100,
            icmp_burst: 50,
            reply_to_ping: true,
        }
    }
}

/// Interface-wide counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub rx_frames: u64,
    pub tx_frames: u64,
    /// Frames the device refused (queue full).
    pub tx_dropped: u64,
    /// Packets dropped as malformed.
    pub rx_malformed: u64,
    /// Packets dropped for a bad checksum.
    pub rx_checksum: u64,
    /// Packets not for us, or for protocols we do not speak.
    pub rx_ignored: u64,
    pub arp_requests_out: u64,
    pub arp_replies_out: u64,
    pub ndp_solicits_out: u64,
    pub ndp_adverts_out: u64,
    pub icmp_out: u64,
    pub icmp_rate_limited: u64,
    pub tcp_rst_out: u64,
    /// SYNs dropped because a listen queue was full.
    pub syn_dropped: u64,
    /// UDP datagrams to a closed port.
    pub udp_no_port: u64,
    /// Packets dropped because the internal loopback queue was full.
    pub loop_dropped: u64,
    /// Packets delivered through the internal loopback queue (to
    /// 127.0.0.0/8, ::1 and the stack's own addresses).
    pub loop_packets: u64,
    pub reassembly: ReassemblyStats,
}

/// A socket's boolean options ([`Stack::flags`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SocketFlags {
    pub reuse_addr: bool,
    pub keepalive: bool,
    pub nodelay: bool,
    pub broadcast: bool,
}

/// TCP connection details (`TCP_INFO`-like), for diagnostics and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpInfo {
    pub state: TcpState,
    pub stats: TcpStats,
    pub srtt: Option<Duration>,
    pub cwnd: usize,
    pub send_mss: usize,
    /// (send, receive) window scale, if negotiated.
    pub window_scales: Option<(u8, u8)>,
    pub timestamps: bool,
    pub send_queued: usize,
    pub recv_queued: usize,
}

#[derive(Debug)]
enum Sock {
    Tcp(Box<Tcb>),
    Dgram(DgramSocket),
}

#[derive(Debug)]
struct Entry {
    generation: u32,
    /// Owned by the user (has a live handle). Unaccepted and closed
    /// connections are not.
    user: bool,
    domain: Domain,
    reuse_addr: bool,
    sock: Sock,
    /// Readiness and queued input as [`Stack::take_changed`] last reported
    /// them.
    seen: Option<(Readiness, usize)>,
}

/// Where a packet goes next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hop {
    /// To ourselves.
    Loop,
    /// Straight to a link-layer address (broadcast, multicast).
    Mac(MacAddr),
    Neighbor4(Ipv4Addr),
    Neighbor6(Ipv6Addr),
}

/// Packets held in the internal loopback queue.
const LOOP_QUEUE: usize = 1024;
/// Frames taken from the device per poll.
const RX_BUDGET: usize = 256;
/// Loopback packets processed per poll.
const LOOP_BUDGET: usize = 1024;
/// Segments one TCP connection may send per poll.
const TCP_BUDGET: usize = 64;
/// Bytes of the offending packet quoted in ICMPv4 errors (RFC 1812 4.3.2.3:
/// as much as fits in 576 bytes).
const ICMP4_QUOTE: usize = 576 - 20 - 8;
/// Same for ICMPv6 (RFC 4443 2.4(c): up to the minimum MTU).
const ICMP6_QUOTE: usize = 1280 - 40 - 8;

/// A TCP/IP stack over one network device.
#[derive(Debug)]
pub struct Stack {
    cfg: Config,
    mac: MacAddr,
    mtu: usize,
    ipv6_ll: Ipv6Addr,
    arp: NeighborCache<Ipv4Addr>,
    ndp: NeighborCache<Ipv6Addr>,
    frag: Reassembler,
    loopq: VecDeque<Vec<u8>>,
    sockets: Vec<Option<Entry>>,
    generations: Vec<u32>,
    now: Instant,
    rx_buf: Vec<u8>,
    tx_buf: Vec<u8>,
    seg_buf: Vec<u8>,
    frame_buf: Vec<u8>,
    /// For frames sent while `frame_buf` is in use (neighbour requests
    /// made from inside `output_ip`, packets released by a reply).
    aux_buf: Vec<u8>,
    ip_ident: u16,
    next_port: u32,
    icmp_tokens: u64,
    icmp_refill: Instant,
    stats: Stats,
    /// TTL and TOS of the packet being sent (a socket's own options while
    /// its output runs).
    pkt_ttl: Option<u8>,
    pkt_tos: u8,
    #[cfg(test)]
    pub(crate) isn_override: Option<u32>,
}

fn sock_addr_v4(ip: Ipv4Addr, port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(ip), port)
}

impl Stack {
    /// A stack for a device with MAC address `mac` and MTU `mtu`.
    pub fn new(mac: MacAddr, mtu: usize, cfg: Config) -> Stack {
        let mtu = mtu.clamp(ipv4::MIN_MTU, ipv4::MAX_PACKET_LEN);
        let frame = (mtu + ethernet::HEADER_LEN).max(ethernet::MIN_FRAME_LEN);
        Stack {
            cfg,
            mac,
            mtu,
            ipv6_ll: ipv6::link_local_from_mac(mac.0),
            arp: NeighborCache::new(cfg.neighbor),
            ndp: NeighborCache::new(cfg.neighbor),
            frag: Reassembler::new(cfg.reassembly),
            loopq: VecDeque::new(),
            sockets: Vec::new(),
            generations: Vec::new(),
            now: Instant::ZERO,
            rx_buf: vec![0; frame],
            tx_buf: vec![0; frame],
            seg_buf: vec![0; frame],
            frame_buf: vec![0; frame],
            aux_buf: vec![0; frame],
            ip_ident: (cfg.secret >> 16) as u16,
            next_port: (cfg.secret >> 32) as u32,
            icmp_tokens: u64::from(cfg.icmp_burst) * 1000,
            icmp_refill: Instant::ZERO,
            stats: Stats::default(),
            pkt_ttl: None,
            pkt_tos: 0,
            #[cfg(test)]
            isn_override: None,
        }
    }

    /// A stack for `dev`, taking its MAC address and MTU.
    pub fn for_device<D: NetDevice>(dev: &D, cfg: Config) -> Stack {
        Stack::new(dev.mac(), dev.mtu(), cfg)
    }

    /// Counters.
    pub fn stats(&self) -> Stats {
        let mut s = self.stats;
        s.reassembly = self.frag.stats;
        s
    }

    /// The configuration.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Change the IPv4 address and router (DHCP, say). Existing
    /// connections keep their addresses.
    pub fn set_ipv4(&mut self, addr: Option<Ipv4Cidr>, gateway: Option<Ipv4Addr>) {
        self.cfg.ipv4 = addr;
        self.cfg.gateway = gateway;
    }

    /// The link-local IPv6 address, if IPv6 is on.
    pub fn ipv6_link_local(&self) -> Option<Ipv6Addr> {
        self.cfg.ipv6.then_some(self.ipv6_ll)
    }

    /// The stack's clock: the latest `now` passed to `poll`.
    pub fn now(&self) -> Instant {
        self.now
    }

    // ---- driving -----------------------------------------------------

    /// Receive, run timers, transmit. Returns when to call again: `None`
    /// means only when something arrives or a socket call is made; a time
    /// at or before `now` means right away.
    pub fn poll<D: NetDevice>(&mut self, dev: &mut D, now: Instant) -> Option<Instant> {
        self.now = self.now.max(now);
        let mut busy = false;

        let mut rx = core::mem::take(&mut self.rx_buf);
        let mut received = 0;
        while received < RX_BUDGET {
            let Some(n) = dev.receive(&mut rx) else { break };
            received += 1;
            self.stats.rx_frames += 1;
            let n = n.min(rx.len());
            self.process_frame(dev, &rx[..n]);
        }
        self.rx_buf = rx;
        if received == RX_BUDGET {
            busy = true;
        }

        for _ in 0..LOOP_BUDGET {
            let Some(p) = self.loopq.pop_front() else { break };
            self.stats.loop_packets += 1;
            self.process_ip(dev, &p, true);
        }

        self.run_neighbor_timers(dev);
        self.run_reassembly_timers(dev);

        let mut blocked = false;
        for idx in 0..self.sockets.len() {
            let Some(mut entry) = self.sockets[idx].take() else { continue };
            match &mut entry.sock {
                Sock::Tcp(tcb) => self.tcp_output(dev, tcb, &mut blocked),
                Sock::Dgram(s) => self.dgram_output(dev, s, &mut blocked),
            }
            self.sockets[idx] = Some(entry);
            self.settle(idx);
        }

        if busy || !self.loopq.is_empty() {
            return Some(self.now);
        }
        self.deadline(blocked)
    }

    /// Whether packets sent to this host wait for the next
    /// [`poll`](Self::poll) (the loopback queue is not empty).
    pub fn loopback_pending(&self) -> bool {
        !self.loopq.is_empty()
    }

    /// When the stack next needs `poll`, as [`poll`](Self::poll) returns it.
    pub fn poll_at(&self) -> Option<Instant> {
        if !self.loopq.is_empty() {
            return Some(self.now);
        }
        self.deadline(false)
    }

    fn deadline(&self, tx_blocked: bool) -> Option<Instant> {
        let now = self.now;
        let mut t = earliest(self.arp.next_deadline(), self.ndp.next_deadline());
        t = earliest(t, self.frag.next_deadline());
        for e in self.sockets.iter().flatten() {
            match &e.sock {
                Sock::Tcp(tcb) => {
                    let ready = !tx_blocked && tcb.wants_to_send() && self.path_ready(tcb.remote.ip());
                    t = earliest(t, if ready { Some(now) } else { tcb.timer_deadline() });
                }
                Sock::Dgram(s) => {
                    if !tx_blocked && s.tx.front().is_some_and(|d| self.path_ready(d.addr.ip())) {
                        t = earliest(t, Some(now));
                    }
                }
            }
        }
        t
    }

    // ---- receive path ------------------------------------------------

    fn process_frame<D: NetDevice>(&mut self, dev: &mut D, buf: &[u8]) {
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

    fn accepts_multicast(&self, mac: MacAddr) -> bool {
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

    fn process_arp<D: NetDevice>(&mut self, dev: &mut D, buf: &[u8]) {
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

    fn process_ip<D: NetDevice>(&mut self, dev: &mut D, packet: &[u8], from_loop: bool) {
        match packet.first().map(|b| b >> 4) {
            Some(4) => self.process_ipv4(dev, packet, from_loop),
            Some(6) if self.cfg.ipv6 || from_loop => self.process_ipv6(dev, packet, from_loop),
            _ => self.stats.rx_malformed += 1,
        }
    }

    fn process_ipv4<D: NetDevice>(&mut self, dev: &mut D, packet: &[u8], from_loop: bool) {
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

    fn deliver_v4<D: NetDevice>(&mut self, dev: &mut D, h: &ipv4::Ipv4Header, payload: &[u8], unicast: bool) {
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

    fn process_ipv6<D: NetDevice>(&mut self, dev: &mut D, packet: &[u8], from_loop: bool) {
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

    fn count_error(&mut self, e: WireError) {
        match e {
            WireError::Checksum => self.stats.rx_checksum += 1,
            WireError::Unsupported => self.stats.rx_ignored += 1,
            WireError::Truncated | WireError::Malformed => self.stats.rx_malformed += 1,
        }
    }

    // ---- ICMP --------------------------------------------------------

    fn icmpv4_input<D: NetDevice>(&mut self, dev: &mut D, ih: &ipv4::Ipv4Header, payload: &[u8], unicast: bool) {
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

    fn icmpv6_input<D: NetDevice>(&mut self, dev: &mut D, ih: &ipv6::Ipv6Header, body: &[u8], multicast: bool) {
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
    fn icmp_allowed(&mut self) -> bool {
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
    fn icmp4_error<D: NetDevice>(
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

    fn icmp6_error<D: NetDevice>(
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
    fn transport_error(&mut self, protocol: u8, src: IpAddr, dst: IpAddr, transport: &[u8], err: IcmpError) {
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

    fn ping_deliver(&mut self, domain: Domain, ident: u16, from: SocketAddr, msg: &[u8]) {
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

    // ---- NDP ---------------------------------------------------------

    fn ndp_solicit_input<D: NetDevice>(&mut self, dev: &mut D, ih: &ipv6::Ipv6Header, m: &icmpv6::Message<'_>) {
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

    fn ndp_advert_input<D: NetDevice>(&mut self, dev: &mut D, ih: &ipv6::Ipv6Header, m: &icmpv6::Message<'_>) {
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

    fn send_ipv6_direct<D: NetDevice>(&mut self, dev: &mut D, mac: MacAddr, src: Ipv6Addr, dst: Ipv6Addr, body: &[u8]) {
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

    fn send_arp_request<D: NetDevice>(&mut self, dev: &mut D, target: Ipv4Addr) {
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

    fn send_neighbor_solicit<D: NetDevice>(&mut self, dev: &mut D, target: Ipv6Addr) {
        let src = self.ipv6_source_for(target);
        let dst = ipv6::solicited_node(target);
        let mut b = [0u8; 32];
        let len = icmpv6::write_neighbor(&mut b, target, icmpv6::OPT_SOURCE_LLADDR, self.mac);
        icmpv6::emit(&mut b, len, icmpv6::NEIGHBOR_SOLICIT, 0, 0, src, dst);
        self.send_ipv6_direct(dev, MacAddr::ipv6_multicast(dst), src, dst, &b[..len]);
        self.stats.ndp_solicits_out += 1;
    }

    fn run_neighbor_timers<D: NetDevice>(&mut self, dev: &mut D) {
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
    fn neighbor_failed(&mut self, packet: &[u8]) {
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

    fn run_reassembly_timers<D: NetDevice>(&mut self, dev: &mut D) {
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

    // ---- send path ---------------------------------------------------

    /// Where packets to `dst` go.
    fn hop(&self, dst: IpAddr) -> Result<Hop, Errno> {
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
    fn path_ready(&self, dst: IpAddr) -> bool {
        match self.hop(dst) {
            Ok(Hop::Neighbor4(nh)) => self.arp.peek(nh).is_some() || !self.arp.has_pending(nh),
            Ok(Hop::Neighbor6(nh)) => self.ndp.peek(nh).is_some() || !self.ndp.has_pending(nh),
            _ => true,
        }
    }

    /// Source address for packets to `dst`.
    fn source_for(&self, dst: IpAddr) -> Result<IpAddr, Errno> {
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

    fn ipv6_source_for(&self, dst: Ipv6Addr) -> Ipv6Addr {
        match self.cfg.ipv6_global {
            Some(g) if !ipv6::is_link_local(dst) && !dst.is_multicast() => g.addr,
            _ => self.ipv6_ll,
        }
    }

    fn our_v6_addrs(&self) -> impl Iterator<Item = Ipv6Addr> + '_ {
        let ll = self.cfg.ipv6.then_some(self.ipv6_ll);
        ll.into_iter().chain(self.cfg.ipv6_global.filter(|_| self.cfg.ipv6).map(|c| c.addr))
    }

    fn is_our_v6(&self, a: Ipv6Addr) -> bool {
        self.our_v6_addrs().any(|x| x == a)
    }

    fn is_local_addr(&self, a: IpAddr) -> bool {
        match a {
            IpAddr::V4(v) => v.is_unspecified() || v.is_loopback() || self.cfg.ipv4.is_some_and(|c| c.addr == v),
            IpAddr::V6(v) => v.is_unspecified() || v.is_loopback() || self.is_our_v6(v),
        }
    }

    /// Largest TCP segment payload towards `dst`.
    fn mss_limit(&self, dst: IpAddr) -> usize {
        let ip = if dst.is_ipv4() { 20 } else { 40 };
        self.mtu - ip - 20
    }

    /// Build and send an IP packet carrying `payload`.
    fn output_ip<D: NetDevice>(
        &mut self,
        dev: &mut D,
        src: IpAddr,
        dst: IpAddr,
        protocol: u8,
        payload: &[u8],
        dont_fragment: bool,
    ) -> Result<(), Errno> {
        let hop = self.hop(dst)?;
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
    fn transmit<D: NetDevice>(&mut self, dev: &mut D, frame: &mut [u8], dst: MacAddr, ethertype: u16, ip_len: usize) {
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
    fn send_frame<D: NetDevice>(&mut self, dev: &mut D, dst: MacAddr, ethertype: u16, packet: &[u8]) {
        let mut frame = core::mem::take(&mut self.aux_buf);
        let n = packet.len().min(frame.len() - ethernet::HEADER_LEN);
        frame[ethernet::HEADER_LEN..ethernet::HEADER_LEN + n].copy_from_slice(&packet[..n]);
        self.transmit(dev, &mut frame, dst, ethertype, n);
        self.aux_buf = frame;
    }

    fn send_tcp<D: NetDevice>(
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

    fn tcp_output<D: NetDevice>(&mut self, dev: &mut D, tcb: &mut Tcb, blocked: &mut bool) {
        tcb.on_timers(self.now);
        (self.pkt_ttl, self.pkt_tos) = (tcb.ttl, tcb.tos);
        self.tcp_output_segments(dev, tcb, blocked);
        (self.pkt_ttl, self.pkt_tos) = (None, 0);
    }

    fn tcp_output_segments<D: NetDevice>(&mut self, dev: &mut D, tcb: &mut Tcb, blocked: &mut bool) {
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

    fn dgram_output<D: NetDevice>(&mut self, dev: &mut D, s: &mut DgramSocket, blocked: &mut bool) {
        (self.pkt_ttl, self.pkt_tos) = (s.ttl, s.tos);
        self.dgram_output_queue(dev, s, blocked);
        (self.pkt_ttl, self.pkt_tos) = (None, 0);
    }

    fn dgram_output_queue<D: NetDevice>(&mut self, dev: &mut D, s: &mut DgramSocket, blocked: &mut bool) {
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
                    let mut b = core::mem::take(&mut self.tx_buf);
                    let n = d.data.len().min(b.len() - udp::HEADER_LEN);
                    b[udp::HEADER_LEN..udp::HEADER_LEN + n].copy_from_slice(&d.data[..n]);
                    let ps = pseudo(src, d.addr.ip(), proto::UDP, udp::HEADER_LEN + n);
                    udp::emit(&mut b, local.port(), d.addr.port(), n, ps);
                    let r = self.output_ip(dev, src, d.addr.ip(), proto::UDP, &b[..udp::HEADER_LEN + n], false);
                    self.tx_buf = b;
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

    // ---- TCP input ---------------------------------------------------

    fn tcp_input<D: NetDevice>(&mut self, dev: &mut D, src: IpAddr, dst: IpAddr, seg: &[u8], unicast: bool) {
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

    fn listener_input<D: NetDevice>(&mut self, dev: &mut D, lidx: usize, src: IpAddr, dst: IpAddr, h: &tcp::TcpHeader) {
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
    fn settle(&mut self, idx: usize) {
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

    fn listen_queue(&mut self, idx: usize) -> Option<&mut crate::tcp::ListenQueue> {
        match self.sockets.get_mut(idx)?.as_mut()?.sock {
            Sock::Tcp(ref mut l) => l.listen.as_mut(),
            Sock::Dgram(_) => None,
        }
    }

    fn find_tcp_conn(&self, local: IpAddr, lport: u16, remote: IpAddr, rport: u16) -> Option<usize> {
        let (l, r) = (SocketAddr::new(local, lport), SocketAddr::new(remote, rport));
        self.sockets.iter().position(|e| match e {
            Some(Entry { sock: Sock::Tcp(t), .. }) => {
                !matches!(t.state, TcpState::Closed | TcpState::Listen) && t.local == l && t.remote == r
            }
            _ => false,
        })
    }

    fn find_listener(&self, dst: IpAddr, port: u16) -> Option<usize> {
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
    fn isn(&self, local: SocketAddr, remote: SocketAddr) -> Seq {
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
    fn udp_input<D: NetDevice>(&mut self, _dev: &mut D, src: IpAddr, dst: IpAddr, seg: &[u8]) -> Option<bool> {
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

    // ---- sockets: bookkeeping ----------------------------------------

    fn free_slots(&self) -> usize {
        let used = self.sockets.iter().filter(|e| e.is_some()).count();
        self.cfg.max_sockets.saturating_sub(used)
    }

    fn insert(&mut self, mut e: Entry) -> Result<usize, Errno> {
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

    fn free(&mut self, idx: usize) {
        self.sockets[idx] = None;
        self.generations[idx] = self.generations[idx].wrapping_add(1);
    }

    fn handle_of(&self, idx: usize) -> SocketHandle {
        SocketHandle { index: idx as u32, generation: self.generations[idx] }
    }

    fn entry(&mut self, h: SocketHandle) -> Result<&mut Entry, Errno> {
        match self.sockets.get_mut(h.index as usize) {
            Some(Some(e)) if e.user && e.generation == h.generation => Ok(e),
            _ => Err(Errno::EBADF),
        }
    }

    fn entry_ref(&self, h: SocketHandle) -> Result<&Entry, Errno> {
        match self.sockets.get(h.index as usize) {
            Some(Some(e)) if e.user && e.generation == h.generation => Ok(e),
            _ => Err(Errno::EBADF),
        }
    }

    /// Bound ports of `protocol` (TCP, UDP or ICMP sockets) and the socket
    /// holding them, for conflict checks.
    fn bound(&self) -> impl Iterator<Item = (usize, Protocol, SocketAddr, bool, Option<TcpState>)> + '_ {
        self.sockets.iter().enumerate().filter_map(|(i, e)| {
            let e = e.as_ref()?;
            match &e.sock {
                Sock::Tcp(t) if t.local.port() != 0 => Some((i, Protocol::Tcp, t.local, e.reuse_addr, Some(t.state))),
                Sock::Dgram(s) => s.local.map(|l| (i, s.proto, l, e.reuse_addr, None)),
                _ => None,
            }
        })
    }

    fn port_in_use(&self, p: Protocol, port: u16) -> bool {
        self.bound().any(|(_, bp, a, _, _)| bp == p && a.port() == port)
    }

    /// Would binding `addr` conflict with an existing socket?
    fn bind_conflict(&self, me: usize, p: Protocol, addr: SocketAddr, reuse: bool) -> bool {
        self.bound().any(|(i, bp, a, other_reuse, state)| {
            if i == me || bp != p || a.port() != addr.port() || a.is_ipv4() != addr.is_ipv4() {
                return false;
            }
            let overlap = a.ip() == addr.ip() || a.ip().is_unspecified() || addr.ip().is_unspecified();
            if !overlap {
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

    fn ephemeral_port(&mut self, p: Protocol) -> Result<u16, Errno> {
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

    // ---- sockets: the API --------------------------------------------

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
        let idx = self.insert(Entry { generation: 0, user: true, domain, reuse_addr: false, sock, seen: None })?;
        Ok(self.handle_of(idx))
    }

    fn protocol_of(e: &Entry) -> Protocol {
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
            let listener_conflict = self.bound().any(|(i, bp, a, _, s)| {
                i != h.index as usize
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
        match &mut self.entry(h)?.sock {
            Sock::Tcp(t) => {
                if matches!(t.state, TcpState::Closed | TcpState::Listen) {
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
            // Connections nobody accepted are reset.
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
        if done {
            self.free(idx);
        } else {
            self.settle(idx);
        }
        Ok(())
    }

    /// Readiness flags of a socket.
    pub fn readiness(&self, h: SocketHandle) -> Result<Readiness, Errno> {
        Ok(Self::readiness_of(self.entry_ref(h)?))
    }

    fn readiness_of(e: &Entry) -> Readiness {
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
    fn signature(e: &Entry) -> (Readiness, usize) {
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

fn hash_addr(h: &mut SipHasher, a: SocketAddr) {
    match a.ip() {
        IpAddr::V4(v) => h.write(&v.octets()),
        IpAddr::V6(v) => h.write(&v.octets()),
    };
    h.write(&a.port().to_be_bytes());
}

fn pseudo(src: IpAddr, dst: IpAddr, protocol: u8, len: usize) -> checksum::Sum {
    match (src, dst) {
        (IpAddr::V4(s), IpAddr::V4(d)) => checksum::pseudo_v4(s, d, protocol, len),
        (IpAddr::V6(s), IpAddr::V6(d)) => checksum::pseudo_v6(s, d, protocol, len),
        _ => checksum::Sum::default(),
    }
}

/// The start of an offending IPv4 packet, for an ICMP error.
fn quote_v4(h: &ipv4::Ipv4Header, payload: &[u8]) -> Vec<u8> {
    let n = payload.len().min(ICMP4_QUOTE - 20);
    let mut q = vec![0u8; 20 + n];
    let mut hh = *h;
    hh.header_len = 20;
    hh.total_len = 20 + payload.len();
    ipv4::emit(&mut q, &hh);
    q[20..].copy_from_slice(&payload[..n]);
    q
}

/// The start of an offending IPv6 packet, for an ICMPv6 error.
fn quote_v6(h: &ipv6::Ipv6Header, payload: &[u8]) -> Vec<u8> {
    let n = payload.len().min(ICMP6_QUOTE - 40);
    let mut q = vec![0u8; 40 + n];
    ipv6::emit(&mut q, h);
    q[40..].copy_from_slice(&payload[..n]);
    q
}

/// The IPv4 header and transport bytes quoted in an ICMP error. Only the
/// header's length fields are checked: the quote is usually cut short.
fn parse_quoted_v4(data: &[u8]) -> Result<(ipv4::Ipv4Header, &[u8]), WireError> {
    if data.len() < 20 || data[0] >> 4 != 4 {
        return Err(WireError::Malformed);
    }
    let hl = usize::from(data[0] & 0xf) * 4;
    if hl < 20 || data.len() < hl + 8 {
        return Err(WireError::Truncated);
    }
    let h = ipv4::Ipv4Header {
        header_len: hl,
        tos: data[1],
        total_len: usize::from(u16::from_be_bytes([data[2], data[3]])),
        ident: u16::from_be_bytes([data[4], data[5]]),
        dont_fragment: false,
        more_fragments: false,
        frag_offset: 0,
        ttl: data[8],
        protocol: data[9],
        src: Ipv4Addr::new(data[12], data[13], data[14], data[15]),
        dst: Ipv4Addr::new(data[16], data[17], data[18], data[19]),
    };
    Ok((h, &data[hl..]))
}
