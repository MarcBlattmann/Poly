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

mod api;
mod icmp;
mod ndp;
mod options;
mod rx;
mod sockets;
mod transport;
mod tx;

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
    /// `SO_REUSEPORT`: sockets that all set it may bind the same address
    /// and port (bind's spec sheet); the first one found receives.
    reuse_port: bool,
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
