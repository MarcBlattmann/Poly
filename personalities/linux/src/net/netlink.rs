//! `AF_NETLINK` / `NETLINK_ROUTE` (netlink(7), rtnetlink(7)): the dumps
//! of links, addresses and routes that `ip addr`, `ip link`, `ip route`
//! and the C library's `getifaddrs`, `if_nameindex` and `getaddrinfo`
//! (`AI_ADDRCONFIG`) ask for, and the two changes a sandbox makes to the
//! loopback device of its new network namespace (ADR 0178).
//!
//! [`NetlinkSocket`] is the protocol state of one socket: its port id and
//! the reply datagrams waiting to be read. A datagram the program sends
//! is a sequence of requests ([`NetlinkSocket::request`]); each gets its
//! answer from the devices of the socket's network namespace, which the
//! kernel provides as a [`LinkEnv`]:
//!
//! - `RTM_GETLINK`, `RTM_GETADDR`, `RTM_GETROUTE` with `NLM_F_DUMP`: one
//!   `RTM_NEW*` message per object (`NLM_F_MULTI`), then `NLMSG_DONE`;
//!   without `NLM_F_DUMP`, `RTM_GETLINK` answers for the link the request
//!   names (by index or `IFLA_IFNAME`; `ENODEV` otherwise).
//! - `RTM_NEWLINK` and `RTM_SETLINK` for a link named by `ifi_index` (or
//!   `IFLA_IFNAME`): the link goes up or down as `IFF_UP` in `ifi_flags`
//!   says when `ifi_change` has that bit (or is 0: rtnetlink(7) reserves
//!   the field); `ENODEV` for a link that is not there. `RTM_NEWADDR` of
//!   127.0.0.1/8 on the loopback device gives it that address (`EEXIST`
//!   if it has it).
//! - A request that changes something needs `CAP_NET_ADMIN` in the user
//!   namespace that owns the network namespace ([`LinkEnv::admin`]):
//!   without it the answer is `-EPERM` (the netns_loopback sheet), with
//!   it `-EOPNOTSUPP` for the changes Poly does not make. Unknown types
//!   get `-EOPNOTSUPP`.
//! - `NLM_F_ACK` on a request that succeeded gets an `NLMSG_ERROR` with
//!   error 0 (netlink(7)); a failed one gets its error with or without
//!   the flag.
//!
//! Routes are derived from the addresses and gateways: a subnet route per
//! address (main table, `RTPROT_KERNEL`, scope link, the address as
//! preferred source; `127.0.0.0/8` lives in the local table as Linux
//! documents loopback), a local route per address (local table,
//! `RTN_LOCAL`, scope host) and a default route per gateway
//! (`RTPROT_BOOT`). Reply datagrams carry at most [`DATAGRAM`] bytes of
//! messages (at least one message). Multicast groups can be joined but no
//! notifications are sent for the changes above.
//!
//! Layouts and constants from `linux/netlink.h`, `linux/rtnetlink.h`,
//! `linux/if_link.h`, `linux/if_addr.h`, `linux/if.h`, `linux/if_arp.h`.

use crate::errno::Errno;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::net::IpAddr;

pub const NLMSG_HDRLEN: usize = 16;
pub const NLMSG_NOOP: u16 = 1;
pub const NLMSG_ERROR: u16 = 2;
pub const NLMSG_DONE: u16 = 3;
pub const NLMSG_MIN_TYPE: u16 = 0x10;

pub const NLM_F_REQUEST: u16 = 0x1;
pub const NLM_F_MULTI: u16 = 0x2;
pub const NLM_F_ACK: u16 = 0x4;
pub const NLM_F_ROOT: u16 = 0x100;
pub const NLM_F_MATCH: u16 = 0x200;
pub const NLM_F_DUMP: u16 = NLM_F_ROOT | NLM_F_MATCH;

pub const RTM_NEWLINK: u16 = 16;
pub const RTM_GETLINK: u16 = 18;
pub const RTM_SETLINK: u16 = 19;
pub const RTM_NEWADDR: u16 = 20;
pub const RTM_GETADDR: u16 = 22;
pub const RTM_NEWROUTE: u16 = 24;
pub const RTM_GETROUTE: u16 = 26;

pub const IFLA_ADDRESS: u16 = 1;
pub const IFLA_BROADCAST: u16 = 2;
pub const IFLA_IFNAME: u16 = 3;
pub const IFLA_MTU: u16 = 4;
pub const IFLA_STATS: u16 = 7;
pub const IFLA_TXQLEN: u16 = 13;
pub const IFLA_OPERSTATE: u16 = 16;
pub const IFLA_LINKMODE: u16 = 17;
pub const IFLA_STATS64: u16 = 23;

pub const IFA_ADDRESS: u16 = 1;
pub const IFA_LOCAL: u16 = 2;
pub const IFA_LABEL: u16 = 3;
pub const IFA_BROADCAST: u16 = 4;
pub const IFA_CACHEINFO: u16 = 6;
pub const IFA_FLAGS: u16 = 8;
pub const IFA_F_PERMANENT: u32 = 0x80;

pub const RTA_DST: u16 = 1;
pub const RTA_OIF: u16 = 4;
pub const RTA_GATEWAY: u16 = 5;
pub const RTA_PRIORITY: u16 = 6;
pub const RTA_PREFSRC: u16 = 7;
pub const RTA_TABLE: u16 = 15;

pub const RTN_UNICAST: u8 = 1;
pub const RTN_LOCAL: u8 = 2;
pub const RTPROT_KERNEL: u8 = 2;
pub const RTPROT_BOOT: u8 = 3;
pub const RT_SCOPE_UNIVERSE: u8 = 0;
pub const RT_SCOPE_LINK: u8 = 253;
pub const RT_SCOPE_HOST: u8 = 254;
pub const RT_TABLE_MAIN: u8 = 254;
pub const RT_TABLE_LOCAL: u8 = 255;

pub const IFF_UP: u32 = 1 << 0;
pub const IFF_BROADCAST: u32 = 1 << 1;
pub const IFF_LOOPBACK: u32 = 1 << 3;
pub const IFF_RUNNING: u32 = 1 << 6;
pub const IFF_MULTICAST: u32 = 1 << 12;
pub const IFF_LOWER_UP: u32 = 1 << 16;
pub const ARPHRD_ETHER: u16 = 1;
pub const ARPHRD_LOOPBACK: u16 = 772;
pub const IF_OPER_UNKNOWN: u8 = 0;
pub const IF_OPER_DOWN: u8 = 2;
pub const IF_OPER_UP: u8 = 6;

const AF_UNSPEC: u8 = 0;
const AF_INET: u8 = 2;
const AF_INET6: u8 = 10;

/// The most bytes of messages in one reply datagram.
pub const DATAGRAM: usize = 8192;

/// Counters of a link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub rx_dropped: u64,
    pub tx_dropped: u64,
}

/// A network interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub index: u32,
    pub name: Vec<u8>,
    pub loopback: bool,
    pub mac: [u8; 6],
    pub mtu: u32,
    /// Administratively up (`IFF_UP`).
    pub up: bool,
    /// Carrier (`IFF_RUNNING`, `IFF_LOWER_UP`).
    pub carrier: bool,
    pub stats: LinkStats,
}

impl Link {
    /// The device's flags (`IFF_*`, `linux/if.h`): `IFF_UP` when it is
    /// administratively up, `IFF_RUNNING` and `IFF_LOWER_UP` when it is up
    /// and has a carrier, and `IFF_LOOPBACK` for `lo` or `IFF_BROADCAST |
    /// IFF_MULTICAST` for an Ethernet device. A loopback device that is
    /// down has `IFF_LOOPBACK` only (0x8), one that is up 0x49 in the 16
    /// bits `SIOCGIFFLAGS` reports.
    pub fn flags(&self) -> u32 {
        let mut f = 0;
        if self.up {
            f |= IFF_UP;
            if self.carrier {
                f |= IFF_RUNNING | IFF_LOWER_UP;
            }
        }
        f | if self.loopback { IFF_LOOPBACK } else { IFF_BROADCAST | IFF_MULTICAST }
    }
}

/// An address of an interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IfAddr {
    pub index: u32,
    pub addr: IpAddr,
    pub prefix_len: u8,
}

/// What the kernel reports: links, their addresses, and default routers
/// with the interface they are on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interfaces {
    pub links: Vec<Link>,
    pub addrs: Vec<IfAddr>,
    pub gateways: Vec<(IpAddr, u32)>,
}

/// The devices of the network namespace a socket's requests act on.
pub trait LinkEnv {
    /// The links, their addresses and the default routers now.
    fn interfaces(&mut self) -> Interfaces;

    /// Whether the sender may change them: `CAP_NET_ADMIN` in the user
    /// namespace that owns the network namespace.
    fn admin(&self) -> bool;

    /// Bring link `index` up or down (`ENODEV` if there is none,
    /// `EOPNOTSUPP` for a device that cannot change).
    fn set_up(&mut self, index: u32, up: bool) -> Result<(), Errno>;

    /// Give the loopback device `index` the address 127.0.0.1/8 (`EEXIST`
    /// if it has it).
    fn add_loopback_address(&mut self, index: u32) -> Result<(), Errno>;
}

/// A snapshot as its own environment: every sender may change it (tests,
/// and callers that only read).
impl LinkEnv for Interfaces {
    fn interfaces(&mut self) -> Interfaces {
        self.clone()
    }

    fn admin(&self) -> bool {
        true
    }

    fn set_up(&mut self, index: u32, up: bool) -> Result<(), Errno> {
        let link = self.links.iter_mut().find(|l| l.index == index).ok_or(Errno::ENODEV)?;
        link.up = up;
        Ok(())
    }

    fn add_loopback_address(&mut self, index: u32) -> Result<(), Errno> {
        let lo = IpAddr::V4(core::net::Ipv4Addr::LOCALHOST);
        if self.addrs.iter().any(|a| a.index == index && a.addr == lo) {
            return Err(Errno::EEXIST);
        }
        self.addrs.push(IfAddr { index, addr: lo, prefix_len: 8 });
        Ok(())
    }
}

/// A route, as `RTM_NEWROUTE` describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub dst: IpAddr,
    pub dst_len: u8,
    pub gateway: Option<IpAddr>,
    pub prefsrc: Option<IpAddr>,
    pub oif: u32,
    pub table: u8,
    pub protocol: u8,
    pub scope: u8,
    pub kind: u8,
    pub priority: Option<u32>,
}

fn family_of(a: &IpAddr) -> u8 {
    if a.is_ipv4() { AF_INET } else { AF_INET6 }
}

fn octets(a: &IpAddr) -> Vec<u8> {
    match a {
        IpAddr::V4(v) => v.octets().to_vec(),
        IpAddr::V6(v) => v.octets().to_vec(),
    }
}

/// `a` with all but the first `len` bits cleared.
fn network(a: IpAddr, len: u8) -> IpAddr {
    match a {
        IpAddr::V4(v) => {
            let m = if len == 0 { 0 } else { u32::MAX << (32 - u32::from(len.min(32))) };
            IpAddr::V4((u32::from(v) & m).into())
        }
        IpAddr::V6(v) => {
            let m = if len == 0 { 0 } else { u128::MAX << (128 - u32::from(len.min(128))) };
            IpAddr::V6((u128::from(v) & m).into())
        }
    }
}

impl Interfaces {
    /// The routing table derived from the addresses and gateways (see the
    /// module documentation).
    pub fn routes(&self) -> Vec<Route> {
        let mut out = Vec::new();
        for a in &self.addrs {
            let loopback = self.links.iter().any(|l| l.index == a.index && l.loopback);
            let subnet = network(a.addr, a.prefix_len);
            let v4_loop = loopback && a.addr.is_ipv4();
            out.push(Route {
                dst: subnet,
                dst_len: a.prefix_len,
                gateway: None,
                prefsrc: a.addr.is_ipv4().then_some(a.addr),
                oif: a.index,
                table: if v4_loop { RT_TABLE_LOCAL } else { RT_TABLE_MAIN },
                protocol: RTPROT_KERNEL,
                scope: if v4_loop { RT_SCOPE_HOST } else { RT_SCOPE_LINK },
                kind: if v4_loop { RTN_LOCAL } else { RTN_UNICAST },
                priority: a.addr.is_ipv6().then_some(256),
            });
            if !(v4_loop && a.prefix_len < 32) {
                out.push(Route {
                    dst: a.addr,
                    dst_len: if a.addr.is_ipv4() { 32 } else { 128 },
                    gateway: None,
                    prefsrc: a.addr.is_ipv4().then_some(a.addr),
                    oif: a.index,
                    table: RT_TABLE_LOCAL,
                    protocol: RTPROT_KERNEL,
                    scope: RT_SCOPE_HOST,
                    kind: RTN_LOCAL,
                    priority: None,
                });
            }
        }
        for &(gw, index) in &self.gateways {
            let any = if gw.is_ipv4() { IpAddr::V4(0.into()) } else { IpAddr::V6(0u128.into()) };
            out.push(Route {
                dst: any,
                dst_len: 0,
                gateway: Some(gw),
                prefsrc: None,
                oif: index,
                table: RT_TABLE_MAIN,
                protocol: RTPROT_BOOT,
                scope: RT_SCOPE_UNIVERSE,
                kind: RTN_UNICAST,
                priority: None,
            });
        }
        out
    }
}

/// `NLMSG_ALIGN` / `RTA_ALIGN`: 4 bytes.
const fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Builds one netlink message.
struct Msg {
    b: Vec<u8>,
}

impl Msg {
    fn new(ty: u16, flags: u16, seq: u32, port: u32) -> Msg {
        let mut b = Vec::with_capacity(256);
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&ty.to_le_bytes());
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&seq.to_le_bytes());
        b.extend_from_slice(&port.to_le_bytes());
        Msg { b }
    }

    fn put(&mut self, bytes: &[u8]) -> &mut Msg {
        self.b.extend_from_slice(bytes);
        self
    }

    /// An attribute (`struct rtattr` and its data, padded).
    fn attr(&mut self, ty: u16, data: &[u8]) -> &mut Msg {
        let len = (4 + data.len()) as u16;
        self.b.extend_from_slice(&len.to_le_bytes());
        self.b.extend_from_slice(&ty.to_le_bytes());
        self.b.extend_from_slice(data);
        self.b.resize(align4(self.b.len()), 0);
        self
    }

    fn finish(mut self) -> Vec<u8> {
        self.b.resize(align4(self.b.len()), 0);
        let len = self.b.len() as u32;
        self.b[0..4].copy_from_slice(&len.to_le_bytes());
        self.b
    }
}

/// A request's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Header {
    len: usize,
    ty: u16,
    flags: u16,
    seq: u32,
}

fn link_message(l: &Link, flags: u16, seq: u32, port: u32) -> Vec<u8> {
    let f = l.flags();
    let mut m = Msg::new(RTM_NEWLINK, flags, seq, port);
    m.put(&[AF_UNSPEC, 0]);
    m.put(&(if l.loopback { ARPHRD_LOOPBACK } else { ARPHRD_ETHER }).to_le_bytes());
    m.put(&(l.index as i32).to_le_bytes());
    m.put(&f.to_le_bytes());
    m.put(&0u32.to_le_bytes());
    let mut name = l.name.clone();
    name.push(0);
    m.attr(IFLA_IFNAME, &name);
    m.attr(IFLA_TXQLEN, &super::dev::TXQLEN.to_le_bytes());
    let oper = if !l.up {
        IF_OPER_DOWN
    } else if l.loopback {
        IF_OPER_UNKNOWN
    } else if l.carrier {
        IF_OPER_UP
    } else {
        IF_OPER_DOWN
    };
    m.attr(IFLA_OPERSTATE, &[oper]);
    m.attr(IFLA_LINKMODE, &[0]);
    m.attr(IFLA_MTU, &l.mtu.to_le_bytes());
    m.attr(IFLA_ADDRESS, &l.mac);
    m.attr(IFLA_BROADCAST, if l.loopback { &[0; 6] } else { &[0xff; 6] });
    let s = &l.stats;
    let fields =
        [s.rx_packets, s.tx_packets, s.rx_bytes, s.tx_bytes, s.rx_errors, s.tx_errors, s.rx_dropped, s.tx_dropped];
    // struct rtnl_link_stats: 24 __u32; struct rtnl_link_stats64: the
    // same counters as __u64 (the first eight are filled in).
    let mut st32 = [0u8; 96];
    let mut st64 = [0u8; 192];
    for (i, v) in fields.iter().enumerate() {
        st32[i * 4..i * 4 + 4].copy_from_slice(&(*v as u32).to_le_bytes());
        st64[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    m.attr(IFLA_STATS, &st32);
    m.attr(IFLA_STATS64, &st64);
    m.finish()
}

fn addr_message(a: &IfAddr, links: &[Link], flags: u16, seq: u32, port: u32) -> Vec<u8> {
    let link = links.iter().find(|l| l.index == a.index);
    let loopback = link.is_some_and(|l| l.loopback);
    let scope = match a.addr {
        IpAddr::V4(v) if v.is_loopback() => RT_SCOPE_HOST,
        IpAddr::V6(v) if v.is_loopback() => RT_SCOPE_HOST,
        IpAddr::V6(v) if v.segments()[0] & 0xffc0 == 0xfe80 => RT_SCOPE_LINK,
        _ => RT_SCOPE_UNIVERSE,
    };
    let mut m = Msg::new(RTM_NEWADDR, flags, seq, port);
    m.put(&[family_of(&a.addr), a.prefix_len, IFA_F_PERMANENT as u8, scope]);
    m.put(&a.index.to_le_bytes());
    let bytes = octets(&a.addr);
    m.attr(IFA_ADDRESS, &bytes);
    if let IpAddr::V4(v) = a.addr {
        m.attr(IFA_LOCAL, &bytes);
        if a.prefix_len < 31 && !loopback {
            let m32 = if a.prefix_len == 0 { 0 } else { u32::MAX << (32 - u32::from(a.prefix_len)) };
            let b = u32::from(v) | !m32;
            m.attr(IFA_BROADCAST, &b.to_be_bytes());
        }
        if let Some(l) = link {
            let mut name = l.name.clone();
            name.push(0);
            m.attr(IFA_LABEL, &name);
        }
    }
    // struct ifa_cacheinfo: lifetimes "forever" (UINT32_MAX), stamps 0.
    let mut ci = [0u8; 16];
    ci[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    ci[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    m.attr(IFA_CACHEINFO, &ci);
    m.attr(IFA_FLAGS, &IFA_F_PERMANENT.to_le_bytes());
    m.finish()
}

fn route_message(r: &Route, flags: u16, seq: u32, port: u32) -> Vec<u8> {
    let mut m = Msg::new(RTM_NEWROUTE, flags, seq, port);
    m.put(&[family_of(&r.dst), r.dst_len, 0, 0, r.table, r.protocol, r.scope, r.kind]);
    m.put(&0u32.to_le_bytes());
    m.attr(RTA_TABLE, &u32::from(r.table).to_le_bytes());
    if r.dst_len > 0 {
        m.attr(RTA_DST, &octets(&r.dst));
    }
    if let Some(p) = r.priority {
        m.attr(RTA_PRIORITY, &p.to_le_bytes());
    }
    if let Some(s) = r.prefsrc {
        m.attr(RTA_PREFSRC, &octets(&s));
    }
    if let Some(g) = r.gateway {
        m.attr(RTA_GATEWAY, &octets(&g));
    }
    m.attr(RTA_OIF, &r.oif.to_le_bytes());
    m.finish()
}

/// `NLMSG_ERROR` for `req` (`error` 0 is an acknowledgement, which carries
/// only the request's header; an error carries the whole request).
fn error_message(error: i32, req: &[u8], h: &Header, port: u32) -> Vec<u8> {
    let mut m = Msg::new(NLMSG_ERROR, 0, h.seq, port);
    m.put(&error.to_le_bytes());
    if error == 0 {
        m.put(&req[..NLMSG_HDRLEN])
    } else {
        m.put(req)
    };
    m.finish()
}

/// The attributes of a request's payload after its fixed header.
fn attrs(mut b: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    core::iter::from_fn(move || {
        if b.len() < 4 {
            return None;
        }
        let len = usize::from(u16::from_le_bytes([b[0], b[1]]));
        let ty = u16::from_le_bytes([b[2], b[3]]);
        if len < 4 || len > b.len() {
            return None;
        }
        let data = &b[4..len];
        b = &b[align4(len).min(b.len())..];
        Some((ty, data))
    })
}

/// The link a `struct ifinfomsg` request names: by `ifi_index`, or by
/// `IFLA_IFNAME` when the index is 0.
fn named_link<'a>(body: &[u8], ifs: &'a Interfaces) -> Option<&'a Link> {
    let index = body.get(4..8).map(|i| i32::from_le_bytes([i[0], i[1], i[2], i[3]])).unwrap_or(0);
    let name = attrs(body.get(16..).unwrap_or(&[]))
        .find(|(t, _)| *t == IFLA_IFNAME)
        .map(|(_, d)| d.split(|&c| c == 0).next().unwrap_or(d));
    ifs.links.iter().find(|l| (index > 0 && l.index == index as u32) || (index <= 0 && name == Some(&l.name[..])))
}

/// A request of type `ty` that changes something (see the module
/// documentation), with `body` after the netlink header.
fn change(ty: u16, body: &[u8], env: &mut dyn LinkEnv) -> Result<(), Errno> {
    if !env.admin() {
        return Err(Errno::EPERM);
    }
    let ifs = env.interfaces();
    match ty {
        RTM_NEWLINK | RTM_SETLINK => {
            // struct ifinfomsg: family, pad, type, index, flags, change.
            let word = |at: usize| body.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            let (Some(flags), Some(mask)) = (word(8), word(12)) else { return Err(Errno::EINVAL) };
            let index = named_link(body, &ifs).ok_or(Errno::ENODEV)?.index;
            if mask == 0 || mask & IFF_UP != 0 {
                env.set_up(index, flags & IFF_UP != 0)?;
            }
            Ok(())
        }
        RTM_NEWADDR => {
            // struct ifaddrmsg: family, prefix length, flags, scope, index.
            let head = body.get(..8).ok_or(Errno::EINVAL)?;
            let index = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
            let link = ifs.links.iter().find(|l| l.index == index).ok_or(Errno::ENODEV)?;
            let addr = attrs(&body[8..]).find(|(t, _)| *t == IFA_LOCAL || *t == IFA_ADDRESS).map(|(_, d)| d);
            if link.loopback && head[0] == AF_INET && head[1] == 8 && addr == Some(&[127, 0, 0, 1][..]) {
                env.add_loopback_address(index)
            } else {
                Err(Errno::EOPNOTSUPP)
            }
        }
        _ => Err(Errno::EOPNOTSUPP),
    }
}

/// One `NETLINK_ROUTE` socket's protocol state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetlinkSocket {
    /// The port id (`nl_pid`), 0 until bound.
    pub port: u32,
    /// Joined multicast groups (`nl_groups` and `NETLINK_ADD_MEMBERSHIP`).
    pub groups: u32,
    /// Reply datagrams waiting to be read.
    queue: VecDeque<Vec<u8>>,
}

impl NetlinkSocket {
    pub fn new() -> NetlinkSocket {
        NetlinkSocket::default()
    }

    /// Move the reply datagrams waiting in `from` to the end of this
    /// socket's (the kernel answers a request on a copy of the socket,
    /// outside its lock, and then queues what came of it).
    pub fn take_replies(&mut self, from: &mut NetlinkSocket) {
        self.queue.append(&mut from.queue);
    }

    /// Bytes of the next datagram, if any.
    pub fn next_len(&self) -> Option<usize> {
        self.queue.front().map(Vec::len)
    }

    /// Take (or with `peek` copy) the next datagram into `buf`; returns
    /// the bytes copied and the datagram's length. `EAGAIN` when none
    /// waits.
    pub fn read(&mut self, buf: &mut [u8], peek: bool) -> Result<(usize, usize), Errno> {
        let d = self.queue.front().ok_or(Errno::EAGAIN)?;
        let n = d.len().min(buf.len());
        buf[..n].copy_from_slice(&d[..n]);
        let full = d.len();
        if !peek {
            self.queue.pop_front();
        }
        Ok((n, full))
    }

    /// Queue `messages` as datagrams of at most [`DATAGRAM`] bytes.
    fn queue_messages(&mut self, messages: Vec<Vec<u8>>) {
        let mut d: Vec<u8> = Vec::new();
        for m in messages {
            if !d.is_empty() && d.len() + m.len() > DATAGRAM {
                self.queue.push_back(core::mem::take(&mut d));
            }
            d.extend_from_slice(&m);
        }
        if !d.is_empty() {
            self.queue.push_back(d);
        }
    }

    /// A datagram sent to the kernel: answer every request in it from
    /// `env` (and make the changes it asks for there). Malformed trailing
    /// bytes are ignored, as `NLMSG_OK` would stop at them.
    pub fn request(&mut self, data: &[u8], env: &mut dyn LinkEnv) {
        let mut at = 0;
        while data.len() - at >= NLMSG_HDRLEN {
            let b = &data[at..];
            let len = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
            if len < NLMSG_HDRLEN || len > b.len() {
                break;
            }
            let h = Header {
                len,
                ty: u16::from_le_bytes([b[4], b[5]]),
                flags: u16::from_le_bytes([b[6], b[7]]),
                seq: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            };
            let req = &b[..len];
            let replies = self.answer(&h, req, env);
            self.queue_messages(replies);
            at += align4(len).min(data.len() - at);
        }
    }

    fn answer(&self, h: &Header, req: &[u8], env: &mut dyn LinkEnv) -> Vec<Vec<u8>> {
        let port = self.port;
        if h.flags & NLM_F_REQUEST == 0 || h.ty < NLMSG_MIN_TYPE {
            return Vec::new();
        }
        let body = &req[NLMSG_HDRLEN..];
        // The types come in fours: new, delete, get, set
        // (`linux/rtnetlink.h`). Everything but a get changes something.
        if (h.ty - NLMSG_MIN_TYPE) % 4 != 2 {
            let done = match change(h.ty, body, env) {
                Ok(()) if h.flags & NLM_F_ACK == 0 => return Vec::new(),
                Ok(()) => 0,
                Err(e) => -i32::from(e.get()),
            };
            return alloc::vec![error_message(done, req, h, port)];
        }
        let ifs = &env.interfaces();
        let dump = h.flags & NLM_F_DUMP != 0;
        let multi = NLM_F_MULTI;
        let done = || {
            let mut m = Msg::new(NLMSG_DONE, NLM_F_MULTI, h.seq, port);
            m.put(&0i32.to_le_bytes());
            m.finish()
        };
        // The family a dump asks for: the first byte of its header (0:
        // all).
        let family = body.first().copied().unwrap_or(AF_UNSPEC);
        let mut out = Vec::new();
        match h.ty {
            RTM_GETLINK if dump => {
                out.extend(ifs.links.iter().map(|l| link_message(l, multi, h.seq, port)));
                out.push(done());
            }
            RTM_GETLINK => {
                // struct ifinfomsg: the index at bytes 4..8; or IFLA_IFNAME.
                match named_link(body, ifs) {
                    Some(l) => {
                        out.push(link_message(l, 0, h.seq, port));
                        if h.flags & NLM_F_ACK != 0 {
                            out.push(error_message(0, req, h, port));
                        }
                    }
                    None => out.push(error_message(-i32::from(Errno::ENODEV.get()), req, h, port)),
                }
            }
            RTM_GETADDR if dump => {
                out.extend(
                    ifs.addrs
                        .iter()
                        .filter(|a| family == AF_UNSPEC || family_of(&a.addr) == family)
                        .map(|a| addr_message(a, &ifs.links, multi, h.seq, port)),
                );
                out.push(done());
            }
            RTM_GETROUTE if dump => {
                out.extend(
                    ifs.routes()
                        .iter()
                        .filter(|r| family == AF_UNSPEC || family_of(&r.dst) == family)
                        .map(|r| route_message(r, multi, h.seq, port)),
                );
                out.push(done());
            }
            _ => out.push(error_message(-i32::from(Errno::EOPNOTSUPP.get()), req, h, port)),
        }
        let _ = h.len;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use core::net::{Ipv4Addr, Ipv6Addr};

    fn ifs() -> Interfaces {
        Interfaces {
            links: vec![
                Link {
                    index: 1,
                    name: b"lo".to_vec(),
                    loopback: true,
                    mac: [0; 6],
                    mtu: 65536,
                    up: true,
                    carrier: true,
                    stats: LinkStats::default(),
                },
                Link {
                    index: 2,
                    name: b"eth0".to_vec(),
                    loopback: false,
                    mac: [0x52, 0x54, 0, 0x12, 0x34, 0x56],
                    mtu: 1500,
                    up: true,
                    carrier: true,
                    stats: LinkStats { rx_packets: 7, ..LinkStats::default() },
                },
            ],
            addrs: vec![
                IfAddr { index: 1, addr: IpAddr::V4(Ipv4Addr::LOCALHOST), prefix_len: 8 },
                IfAddr { index: 1, addr: IpAddr::V6(Ipv6Addr::LOCALHOST), prefix_len: 128 },
                IfAddr { index: 2, addr: IpAddr::V4(Ipv4Addr::new(10, 0, 2, 15)), prefix_len: 24 },
            ],
            gateways: vec![(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2)), 2)],
        }
    }

    fn request(ty: u16, flags: u16, seq: u32, body: &[u8]) -> Vec<u8> {
        let mut m = Msg::new(ty, flags, seq, 0);
        m.put(body);
        m.finish()
    }

    /// Every message in every queued datagram: (type, flags, seq, body).
    fn drain(s: &mut NetlinkSocket) -> Vec<(u16, u16, u32, Vec<u8>)> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; 65536];
        while let Ok((n, full)) = s.read(&mut buf, false) {
            assert_eq!(n, full);
            assert!(n <= DATAGRAM + 4096);
            let mut at = 0;
            while at + NLMSG_HDRLEN <= n {
                let b = &buf[at..n];
                let len = u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize;
                assert!(len >= NLMSG_HDRLEN && len <= b.len() && len.is_multiple_of(4));
                let ty = u16::from_le_bytes([b[4], b[5]]);
                let flags = u16::from_le_bytes([b[6], b[7]]);
                let seq = u32::from_le_bytes(b[8..12].try_into().unwrap());
                let pid = u32::from_le_bytes(b[12..16].try_into().unwrap());
                assert_eq!(pid, s.port);
                out.push((ty, flags, seq, b[NLMSG_HDRLEN..len].to_vec()));
                at += len;
            }
        }
        out
    }

    fn attr_of(body: &[u8], skip: usize, ty: u16) -> Option<Vec<u8>> {
        attrs(&body[skip..]).find(|(t, _)| *t == ty).map(|(_, d)| d.to_vec())
    }

    #[test]
    fn getaddr_dump_lists_every_address_then_done() {
        let mut s = NetlinkSocket { port: 42, ..NetlinkSocket::default() };
        s.request(&request(RTM_GETADDR, NLM_F_REQUEST | NLM_F_DUMP, 7, &[0; 8]), &mut ifs());
        let msgs = drain(&mut s);
        assert_eq!(msgs.len(), 4);
        assert!(msgs[..3].iter().all(|m| m.0 == RTM_NEWADDR && m.1 == NLM_F_MULTI && m.2 == 7));
        assert_eq!((msgs[3].0, msgs[3].1, &msgs[3].3[..]), (NLMSG_DONE, NLM_F_MULTI, &[0u8, 0, 0, 0][..]));
        // 127.0.0.1/8 on lo, scope host.
        let lo = &msgs[0].3;
        assert_eq!((lo[0], lo[1], lo[3]), (AF_INET, 8, RT_SCOPE_HOST));
        assert_eq!(u32::from_le_bytes(lo[4..8].try_into().unwrap()), 1);
        assert_eq!(attr_of(lo, 8, IFA_LOCAL), Some(vec![127, 0, 0, 1]));
        assert_eq!(attr_of(lo, 8, IFA_LABEL), Some(b"lo\0".to_vec()));
        assert_eq!(attr_of(lo, 8, IFA_BROADCAST), None);
        // 10.0.2.15/24 on eth0 with its broadcast address.
        let eth = &msgs[2].3;
        assert_eq!(attr_of(eth, 8, IFA_ADDRESS), Some(vec![10, 0, 2, 15]));
        assert_eq!(attr_of(eth, 8, IFA_BROADCAST), Some(vec![10, 0, 2, 255]));
        // Only IPv6, as getaddrinfo's AI_ADDRCONFIG check may ask.
        s.request(&request(RTM_GETADDR, NLM_F_REQUEST | NLM_F_DUMP, 8, &[AF_INET6, 0, 0, 0, 0, 0, 0, 0]), &mut ifs());
        let msgs = drain(&mut s);
        assert_eq!(msgs.len(), 2);
        assert_eq!(attr_of(&msgs[0].3, 8, IFA_ADDRESS), Some(Ipv6Addr::LOCALHOST.octets().to_vec()));
    }

    #[test]
    fn getlink_dump_and_single_link() {
        let mut s = NetlinkSocket { port: 1, ..NetlinkSocket::default() };
        s.request(&request(RTM_GETLINK, NLM_F_REQUEST | NLM_F_DUMP, 1, &[0; 16]), &mut ifs());
        let msgs = drain(&mut s);
        assert_eq!(msgs.iter().map(|m| m.0).collect::<Vec<_>>(), [RTM_NEWLINK, RTM_NEWLINK, NLMSG_DONE]);
        let eth = &msgs[1].3;
        assert_eq!(u16::from_le_bytes([eth[2], eth[3]]), ARPHRD_ETHER);
        let flags = u32::from_le_bytes(eth[8..12].try_into().unwrap());
        assert_eq!(flags & (IFF_UP | IFF_RUNNING | IFF_LOWER_UP), IFF_UP | IFF_RUNNING | IFF_LOWER_UP);
        assert_eq!(attr_of(eth, 16, IFLA_IFNAME), Some(b"eth0\0".to_vec()));
        assert_eq!(attr_of(eth, 16, IFLA_ADDRESS), Some(vec![0x52, 0x54, 0, 0x12, 0x34, 0x56]));
        assert_eq!(attr_of(eth, 16, IFLA_MTU), Some(1500u32.to_le_bytes().to_vec()));
        assert_eq!(attr_of(eth, 16, IFLA_STATS).map(|s| s.len()), Some(96));
        assert_eq!(attr_of(eth, 16, IFLA_STATS64).map(|s| s[0]), Some(7));
        let lo = &msgs[0].3;
        assert_ne!(u32::from_le_bytes(lo[8..12].try_into().unwrap()) & IFF_LOOPBACK, 0);
        // One link by index, with an acknowledgement.
        let mut body = [0u8; 16];
        body[4..8].copy_from_slice(&2i32.to_le_bytes());
        s.request(&request(RTM_GETLINK, NLM_F_REQUEST | NLM_F_ACK, 5, &body), &mut ifs());
        let msgs = drain(&mut s);
        assert_eq!((msgs[0].0, msgs[0].1), (RTM_NEWLINK, 0));
        assert_eq!(msgs[1].0, NLMSG_ERROR);
        assert_eq!(i32::from_le_bytes(msgs[1].3[0..4].try_into().unwrap()), 0);
        assert_eq!(msgs[1].3.len(), 4 + NLMSG_HDRLEN, "an ack carries only the header");
        // By name.
        let mut m = Msg::new(RTM_GETLINK, NLM_F_REQUEST, 6, 0);
        m.put(&[0; 16]).attr(IFLA_IFNAME, b"lo\0");
        s.request(&m.finish(), &mut ifs());
        let msgs = drain(&mut s);
        assert_eq!(u32::from_le_bytes(msgs[0].3[4..8].try_into().unwrap()), 1);
        // No such link.
        body[4..8].copy_from_slice(&9i32.to_le_bytes());
        s.request(&request(RTM_GETLINK, NLM_F_REQUEST, 7, &body), &mut ifs());
        let msgs = drain(&mut s);
        assert_eq!(msgs[0].0, NLMSG_ERROR);
        assert_eq!(i32::from_le_bytes(msgs[0].3[0..4].try_into().unwrap()), -19);
        assert_eq!(msgs[0].3.len(), 4 + NLMSG_HDRLEN + 16, "an error carries the request");
    }

    #[test]
    fn getroute_dump_has_the_subnet_and_default_routes() {
        let mut s = NetlinkSocket::new();
        s.request(
            &request(RTM_GETROUTE, NLM_F_REQUEST | NLM_F_DUMP, 3, &[AF_INET, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            &mut ifs(),
        );
        let msgs = drain(&mut s);
        let routes: Vec<_> = msgs.iter().filter(|m| m.0 == RTM_NEWROUTE).collect();
        let main: Vec<_> = routes.iter().filter(|m| m.3[4] == RT_TABLE_MAIN).collect();
        assert_eq!(main.len(), 2, "10.0.2.0/24 and the default route");
        let subnet = &main[0].3;
        assert_eq!((subnet[1], subnet[5], subnet[6], subnet[7]), (24, RTPROT_KERNEL, RT_SCOPE_LINK, RTN_UNICAST));
        assert_eq!(attr_of(subnet, 12, RTA_DST), Some(vec![10, 0, 2, 0]));
        assert_eq!(attr_of(subnet, 12, RTA_PREFSRC), Some(vec![10, 0, 2, 15]));
        assert_eq!(attr_of(subnet, 12, RTA_OIF), Some(2u32.to_le_bytes().to_vec()));
        let default = &main[1].3;
        assert_eq!(default[1], 0);
        assert_eq!(attr_of(default, 12, RTA_DST), None);
        assert_eq!(attr_of(default, 12, RTA_GATEWAY), Some(vec![10, 0, 2, 2]));
        assert!(routes.iter().any(|m| m.3[4] == RT_TABLE_LOCAL && m.3[7] == RTN_LOCAL));
        assert_eq!(msgs.last().unwrap().0, NLMSG_DONE);
    }

    #[test]
    fn changes_and_unknown_requests_are_refused() {
        let mut s = NetlinkSocket::new();
        let req = request(RTM_NEWROUTE, NLM_F_REQUEST | NLM_F_ACK, 9, &[0; 12]);
        // Two requests in one datagram, and trailing garbage.
        let mut d = req.clone();
        d.extend(request(RTM_GETADDR, NLM_F_REQUEST | NLM_F_DUMP, 10, &[0; 8]));
        d.extend([1, 2, 3]);
        s.request(&d, &mut ifs());
        let msgs = drain(&mut s);
        assert_eq!(msgs[0].0, NLMSG_ERROR);
        assert_eq!(i32::from_le_bytes(msgs[0].3[0..4].try_into().unwrap()), -95);
        assert_eq!(&msgs[0].3[4..], &req[..]);
        assert_eq!(msgs.last().unwrap().0, NLMSG_DONE);
        // Not a request: ignored.
        s.request(&request(RTM_GETLINK, NLM_F_DUMP, 1, &[0; 16]), &mut ifs());
        assert_eq!(s.next_len(), None);
    }

    /// A netlink message header, an ifinfomsg and nothing else: what
    /// bubblewrap and ip(8) send to bring a link up.
    fn link_request(ty: u16, flags: u16, index: i32, ifflags: u32, change: u32) -> Vec<u8> {
        let mut body = [0u8; 16];
        body[4..8].copy_from_slice(&index.to_le_bytes());
        body[8..12].copy_from_slice(&ifflags.to_le_bytes());
        body[12..16].copy_from_slice(&change.to_le_bytes());
        request(ty, flags, 77, &body)
    }

    /// An environment that refuses changes unless `admin`.
    struct Env {
        devices: crate::net::dev::Devices,
        admin: bool,
    }

    impl LinkEnv for Env {
        fn interfaces(&mut self) -> Interfaces {
            self.devices.interfaces(LinkStats::default())
        }
        fn admin(&self) -> bool {
            self.admin
        }
        fn set_up(&mut self, index: u32, up: bool) -> Result<(), Errno> {
            if index != 1 {
                return Err(Errno::ENODEV);
            }
            self.devices.set_lo_up(up);
            Ok(())
        }
        fn add_loopback_address(&mut self, _: u32) -> Result<(), Errno> {
            self.devices.add_lo_addr()
        }
    }

    /// The netns_loopback sheet's sequence in a new network namespace.
    #[test]
    fn a_new_namespaces_loopback_is_brought_up_over_netlink() {
        let mut env = Env { devices: crate::net::dev::Devices::new(), admin: true };
        let mut s = NetlinkSocket { port: 9, ..NetlinkSocket::default() };
        s.request(&request(RTM_GETLINK, NLM_F_REQUEST | NLM_F_DUMP, 77, &[0; 16]), &mut env);
        let msgs = drain(&mut s);
        assert_eq!(msgs.iter().map(|m| m.0).collect::<Vec<_>>(), [RTM_NEWLINK, NLMSG_DONE], "one link");
        let lo = &msgs[0].3;
        assert_eq!(i32::from_le_bytes(lo[4..8].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(lo[8..12].try_into().unwrap()), IFF_LOOPBACK, "down");
        assert_eq!(attr_of(lo, 16, IFLA_IFNAME), Some(b"lo\0".to_vec()), "attribute length 4 + 3 = 7");
        assert_eq!(attr_of(lo, 16, IFLA_MTU), Some(65536u32.to_le_bytes().to_vec()));
        // No address yet.
        s.request(&request(RTM_GETADDR, NLM_F_REQUEST | NLM_F_DUMP, 77, &[AF_INET, 0, 0, 0, 0, 0, 0, 0]), &mut env);
        assert_eq!(drain(&mut s).iter().map(|m| m.0).collect::<Vec<_>>(), [NLMSG_DONE]);

        // Up: acknowledged with error 0 and the request's header, 36 bytes.
        s.request(&link_request(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, 1, IFF_UP, IFF_UP), &mut env);
        assert_eq!(s.next_len(), Some(36));
        let msgs = drain(&mut s);
        assert_eq!((msgs.len(), msgs[0].0, msgs[0].2), (1, NLMSG_ERROR, 77));
        assert_eq!(i32::from_le_bytes(msgs[0].3[0..4].try_into().unwrap()), 0);
        assert_eq!(u16::from_le_bytes([msgs[0].3[8], msgs[0].3[9]]), RTM_NEWLINK, "the request's header");
        assert!(env.devices.lo_up());
        s.request(&request(RTM_GETLINK, NLM_F_REQUEST | NLM_F_DUMP, 78, &[0; 16]), &mut env);
        let msgs = drain(&mut s);
        let flags = u32::from_le_bytes(msgs[0].3[8..12].try_into().unwrap());
        assert_eq!(flags & 0xffff, 0x49, "IFF_UP | IFF_LOOPBACK | IFF_RUNNING");

        // An unknown index.
        s.request(&link_request(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, 99, IFF_UP, IFF_UP), &mut env);
        let msgs = drain(&mut s);
        assert_eq!((msgs[0].0, i32::from_le_bytes(msgs[0].3[0..4].try_into().unwrap())), (NLMSG_ERROR, -19));

        // The address it now has: 127.0.0.1/8, scope host, labelled lo.
        s.request(&request(RTM_GETADDR, NLM_F_REQUEST | NLM_F_DUMP, 79, &[AF_INET, 0, 0, 0, 0, 0, 0, 0]), &mut env);
        let msgs = drain(&mut s);
        assert_eq!(msgs.iter().map(|m| m.0).collect::<Vec<_>>(), [RTM_NEWADDR, NLMSG_DONE]);
        let a = &msgs[0].3;
        assert_eq!((a[0], a[1], a[3]), (AF_INET, 8, RT_SCOPE_HOST));
        assert_eq!(u32::from_le_bytes(a[4..8].try_into().unwrap()), 1);
        assert_eq!(attr_of(a, 8, IFA_ADDRESS), Some(vec![127, 0, 0, 1]));
        assert_eq!(attr_of(a, 8, IFA_LABEL), Some(b"lo\0".to_vec()));

        // Down again without asking for an acknowledgement: no answer.
        s.request(&link_request(RTM_SETLINK, NLM_F_REQUEST, 1, 0, IFF_UP), &mut env);
        assert_eq!(s.next_len(), None);
        assert!(!env.devices.lo_up());
        // A change mask without IFF_UP leaves the link as it is.
        s.request(&link_request(RTM_NEWLINK, NLM_F_REQUEST, 1, IFF_UP, IFF_MULTICAST), &mut env);
        assert!(!env.devices.lo_up());
        // By name, with a reserved (zero) change mask.
        let mut m = Msg::new(RTM_NEWLINK, NLM_F_REQUEST, 80, 0);
        let mut body = [0u8; 16];
        body[8..12].copy_from_slice(&IFF_UP.to_le_bytes());
        m.put(&body).attr(IFLA_IFNAME, b"lo\0");
        s.request(&m.finish(), &mut env);
        assert!(env.devices.lo_up());
    }

    #[test]
    fn changes_need_cap_net_admin_and_only_some_are_made() {
        let mut env = Env { devices: crate::net::dev::Devices::new(), admin: false };
        let mut s = NetlinkSocket::new();
        // Without the capability: dumps work, changes are EPERM.
        s.request(&request(RTM_GETLINK, NLM_F_REQUEST | NLM_F_DUMP, 1, &[0; 16]), &mut env);
        assert_eq!(drain(&mut s).len(), 2);
        s.request(&link_request(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, 1, IFF_UP, IFF_UP), &mut env);
        let msgs = drain(&mut s);
        assert_eq!((msgs[0].0, i32::from_le_bytes(msgs[0].3[0..4].try_into().unwrap())), (NLMSG_ERROR, -1));
        assert!(!env.devices.lo_up());
        // Refused before the link is looked up, and without NLM_F_ACK too.
        s.request(&link_request(RTM_NEWLINK, NLM_F_REQUEST, 99, IFF_UP, IFF_UP), &mut env);
        assert_eq!(i32::from_le_bytes(drain(&mut s)[0].3[0..4].try_into().unwrap()), -1);
        s.request(&request(RTM_NEWADDR, NLM_F_REQUEST, 2, &[0; 8]), &mut env);
        assert_eq!(i32::from_le_bytes(drain(&mut s)[0].3[0..4].try_into().unwrap()), -1);

        // With it: 127.0.0.1/8 on lo is taken once; other addresses and
        // deletions are not supported.
        env.admin = true;
        let newaddr = |addr: [u8; 4], len: u8, index: u32| {
            let mut m = Msg::new(RTM_NEWADDR, NLM_F_REQUEST | NLM_F_ACK, 3, 0);
            let mut head = [AF_INET, len, 0, RT_SCOPE_HOST, 0, 0, 0, 0];
            head[4..8].copy_from_slice(&index.to_le_bytes());
            m.put(&head).attr(IFA_LOCAL, &addr).attr(IFA_ADDRESS, &addr);
            m.finish()
        };
        let code = |s: &mut NetlinkSocket| i32::from_le_bytes(drain(s)[0].3[0..4].try_into().unwrap());
        s.request(&newaddr([127, 0, 0, 1], 8, 1), &mut env);
        assert_eq!(code(&mut s), 0);
        assert!(!env.devices.lo_up(), "an address does not bring the link up");
        s.request(&newaddr([127, 0, 0, 1], 8, 1), &mut env);
        assert_eq!(code(&mut s), -17);
        s.request(&newaddr([10, 0, 0, 1], 24, 1), &mut env);
        assert_eq!(code(&mut s), -95);
        s.request(&newaddr([127, 0, 0, 1], 8, 5), &mut env);
        assert_eq!(code(&mut s), -19);
        s.request(&link_request(RTM_NEWLINK + 1, NLM_F_REQUEST | NLM_F_ACK, 1, 0, 0), &mut env);
        assert_eq!(code(&mut s), -95, "RTM_DELLINK");
        s.request(&request(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, 4, &[0; 4]), &mut env);
        assert_eq!(code(&mut s), -22, "too short for an ifinfomsg");
    }

    #[test]
    fn big_dumps_span_datagrams_and_reads_truncate() {
        let mut many = ifs();
        for i in 0..100 {
            many.addrs.push(IfAddr { index: 2, addr: IpAddr::V4(Ipv4Addr::new(10, 1, 0, i)), prefix_len: 16 });
        }
        let mut s = NetlinkSocket::new();
        s.request(&request(RTM_GETADDR, NLM_F_REQUEST | NLM_F_DUMP, 1, &[0; 8]), &mut many);
        assert!(s.queue.len() > 1);
        assert!(s.queue.iter().all(|d| d.len() <= DATAGRAM));
        let mut small = [0u8; 10];
        let (n, full) = s.read(&mut small, true).unwrap();
        assert!(n == 10 && full > 10);
        let (n, full) = s.read(&mut small, false).unwrap();
        assert!(n == 10 && full > 10, "the rest of the datagram is gone");
        let msgs = drain(&mut s);
        assert_eq!(msgs.last().unwrap().0, NLMSG_DONE);
    }
}
