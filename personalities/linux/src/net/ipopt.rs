//! IP, IPv6 and TCP socket options an Internet socket keeps (ip(7),
//! ipv6(7), tcp(7)): the int-valued ones that are stored and reported back,
//! multicast group memberships, `IP_MULTICAST_IF`, `IPV6_PKTINFO`,
//! `TCP_CONGESTION`, and the ancillary data a receive returns for
//! `IP_PKTINFO`, `IP_RECVTOS`, `IP_RECVTTL` and their IPv6 counterparts.
//!
//! **What is acted on.** Poly's stack acts on `IP_TOS` / `IPV6_TCLASS` and
//! the TTL options (they are in the packets it sends) and on the receive
//! flags (the ancillary data are built from what each datagram arrived
//! with). Every other option here (`IP_MTU_DISCOVER`, `IP_FREEBIND`,
//! `IP_TRANSPARENT`, the multicast options, group memberships, the TCP
//! tuning options, ...) is checked as the man pages say (`EINVAL` for a
//! value out of range or a short argument), stored, and reported back by
//! `getsockopt` (the documented default if never set); the stack does not
//! change its behaviour for them. In particular a joined multicast group
//! receives nothing yet.
//!
//! Values come from the public headers `linux/in.h`, `linux/in6.h` and
//! `linux/tcp.h`.

use super::abi::{self, Cmsg};
use crate::errno::Errno;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Which sockets an option applies to; others answer `ENOPROTOOPT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// `SOL_IP`: `AF_INET` sockets.
    V4,
    /// `SOL_IPV6`: `AF_INET6` sockets.
    V6,
    /// `SOL_TCP`: TCP sockets.
    Tcp,
}

/// An option whose value is one int, stored as given (after
/// [`normalize`](Self::normalize)) and reported back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredOpt {
    IpPktInfo,
    IpRecvTos,
    IpRecvTtl,
    IpRecvOpts,
    IpRetOpts,
    IpMtuDiscover,
    IpFreebind,
    IpTransparent,
    IpBindAddressNoPort,
    IpMulticastTtl,
    IpMulticastLoop,
    Ipv6MulticastIf,
    Ipv6MulticastHops,
    Ipv6MulticastLoop,
    Ipv6RecvTclass,
    Ipv6RecvPktInfo,
    Ipv6RecvHopLimit,
    Ipv6MtuDiscover,
    Ipv6DontFrag,
    TcpFastopen,
    TcpFastopenConnect,
    TcpQuickack,
    TcpCork,
    TcpDeferAccept,
    TcpUserTimeout,
    TcpNotsentLowat,
}

/// How many [`StoredOpt`]s there are.
const COUNT: usize = 26;

/// `IP_PMTUDISC_OMIT`, the largest value of `IP_MTU_DISCOVER` (ip(7)).
const PMTUDISC_MAX: i32 = 5;
/// `IP_PMTUDISC_WANT`: "use per-route settings", the default.
const PMTUDISC_WANT: i32 = 1;
/// Memberships one socket may hold (ip(7): `IP_MAX_MEMBERSHIPS`).
pub const MAX_MEMBERSHIPS: usize = 20;
/// `TCP_CA_NAME_MAX`: congestion control names are shorter than this.
pub const TCP_CA_NAME_MAX: usize = 16;

impl StoredOpt {
    /// The option for a `(level, optname)` pair, if it is one of these.
    pub fn from_linux(level: i32, name: i32) -> Option<StoredOpt> {
        use StoredOpt::*;
        Some(match (level, name) {
            (abi::SOL_IP, abi::IP_PKTINFO) => IpPktInfo,
            (abi::SOL_IP, abi::IP_RECVTOS) => IpRecvTos,
            (abi::SOL_IP, abi::IP_RECVTTL) => IpRecvTtl,
            (abi::SOL_IP, abi::IP_RECVOPTS) => IpRecvOpts,
            (abi::SOL_IP, abi::IP_RETOPTS) => IpRetOpts,
            (abi::SOL_IP, abi::IP_MTU_DISCOVER) => IpMtuDiscover,
            (abi::SOL_IP, abi::IP_FREEBIND) => IpFreebind,
            (abi::SOL_IP, abi::IP_TRANSPARENT) => IpTransparent,
            (abi::SOL_IP, abi::IP_BIND_ADDRESS_NO_PORT) => IpBindAddressNoPort,
            (abi::SOL_IP, abi::IP_MULTICAST_TTL) => IpMulticastTtl,
            (abi::SOL_IP, abi::IP_MULTICAST_LOOP) => IpMulticastLoop,
            (abi::SOL_IPV6, abi::IPV6_MULTICAST_IF) => Ipv6MulticastIf,
            (abi::SOL_IPV6, abi::IPV6_MULTICAST_HOPS) => Ipv6MulticastHops,
            (abi::SOL_IPV6, abi::IPV6_MULTICAST_LOOP) => Ipv6MulticastLoop,
            (abi::SOL_IPV6, abi::IPV6_RECVTCLASS) => Ipv6RecvTclass,
            (abi::SOL_IPV6, abi::IPV6_RECVPKTINFO) => Ipv6RecvPktInfo,
            (abi::SOL_IPV6, abi::IPV6_RECVHOPLIMIT) => Ipv6RecvHopLimit,
            (abi::SOL_IPV6, abi::IPV6_MTU_DISCOVER) => Ipv6MtuDiscover,
            (abi::SOL_IPV6, abi::IPV6_DONTFRAG) => Ipv6DontFrag,
            (abi::SOL_TCP, abi::TCP_FASTOPEN) => TcpFastopen,
            (abi::SOL_TCP, abi::TCP_FASTOPEN_CONNECT) => TcpFastopenConnect,
            (abi::SOL_TCP, abi::TCP_QUICKACK) => TcpQuickack,
            (abi::SOL_TCP, abi::TCP_CORK) => TcpCork,
            (abi::SOL_TCP, abi::TCP_DEFER_ACCEPT) => TcpDeferAccept,
            (abi::SOL_TCP, abi::TCP_USER_TIMEOUT) => TcpUserTimeout,
            (abi::SOL_TCP, abi::TCP_NOTSENT_LOWAT) => TcpNotsentLowat,
            _ => return None,
        })
    }

    /// The sockets it applies to.
    pub fn scope(self) -> Scope {
        use StoredOpt::*;
        match self {
            IpPktInfo | IpRecvTos | IpRecvTtl | IpRecvOpts | IpRetOpts | IpMtuDiscover | IpFreebind | IpTransparent
            | IpBindAddressNoPort | IpMulticastTtl | IpMulticastLoop => Scope::V4,
            Ipv6MulticastIf | Ipv6MulticastHops | Ipv6MulticastLoop | Ipv6RecvTclass | Ipv6RecvPktInfo
            | Ipv6RecvHopLimit | Ipv6MtuDiscover | Ipv6DontFrag => Scope::V6,
            TcpFastopen | TcpFastopenConnect | TcpQuickack | TcpCork | TcpDeferAccept | TcpUserTimeout
            | TcpNotsentLowat => Scope::Tcp,
        }
    }

    /// Whether the call takes a single byte where the argument is shorter
    /// than an int (ip(7): the `IP_*` options of this kind do).
    pub fn byte_form(self) -> bool {
        self.scope() == Scope::V4
    }

    /// The value of a socket that never set it.
    pub fn default_value(self) -> i32 {
        use StoredOpt::*;
        match self {
            IpMtuDiscover | Ipv6MtuDiscover => PMTUDISC_WANT,
            IpMulticastTtl | IpMulticastLoop | Ipv6MulticastHops | Ipv6MulticastLoop | TcpQuickack => 1,
            _ => 0,
        }
    }

    /// Check a value as the pages say and reduce it to what is kept:
    /// `EINVAL` when it is out of range. Flags become 0 or 1.
    pub fn normalize(self, v: i32) -> Result<i32, Errno> {
        use StoredOpt::*;
        match self {
            // Boolean options: any non-zero value is on.
            IpPktInfo | IpRecvTos | IpRecvTtl | IpRecvOpts | IpRetOpts | IpFreebind | IpTransparent
            | IpBindAddressNoPort | IpMulticastLoop | Ipv6RecvTclass | Ipv6RecvPktInfo | Ipv6RecvHopLimit
            | Ipv6DontFrag | TcpQuickack | TcpCork => Ok(i32::from(v != 0)),
            IpMtuDiscover | Ipv6MtuDiscover => (0..=PMTUDISC_MAX).contains(&v).then_some(v).ok_or(Errno::EINVAL),
            IpMulticastTtl => (0..=255).contains(&v).then_some(v).ok_or(Errno::EINVAL),
            // ipv6(7): -1 is the default hop limit (1 for multicast).
            Ipv6MulticastHops => match v {
                -1 => Ok(1),
                0..=255 => Ok(v),
                _ => Err(Errno::EINVAL),
            },
            // IPV6_MULTICAST_LOOP is strictly 0 or 1.
            Ipv6MulticastLoop | TcpFastopenConnect => (v == 0 || v == 1).then_some(v).ok_or(Errno::EINVAL),
            Ipv6MulticastIf | TcpFastopen | TcpUserTimeout => (v >= 0).then_some(v).ok_or(Errno::EINVAL),
            // tcp(7): a negative time is none.
            TcpDeferAccept => Ok(v.max(0)),
            TcpNotsentLowat => Ok(v),
        }
    }
}

/// `TCP_CONGESTION`: the algorithms an unprivileged process may pick
/// (tcp(7): `tcp_allowed_congestion_control`). The stack runs its own
/// whatever is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Congestion {
    #[default]
    Cubic,
    Reno,
}

impl Congestion {
    pub fn name(self) -> &'static str {
        match self {
            Congestion::Cubic => "cubic",
            Congestion::Reno => "reno",
        }
    }

    /// The algorithm a `TCP_CONGESTION` argument names: its first
    /// `TCP_CA_NAME_MAX - 1` bytes up to a NUL; `ENOENT` for one Poly does
    /// not have, `EINVAL` for an empty argument.
    pub fn parse(arg: &[u8]) -> Result<Congestion, Errno> {
        if arg.is_empty() {
            return Err(Errno::EINVAL);
        }
        let arg = &arg[..arg.len().min(TCP_CA_NAME_MAX - 1)];
        let end = arg.iter().position(|&b| b == 0).unwrap_or(arg.len());
        match &arg[..end] {
            b"cubic" => Ok(Congestion::Cubic),
            b"reno" => Ok(Congestion::Reno),
            _ => Err(Errno::ENOENT),
        }
    }
}

/// A multicast group a socket joined (ip(7) `ip_mreq` / `ip_mreqn`,
/// ipv6(7) `ipv6_mreq`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Membership {
    pub group: IpAddr,
    /// The interface by index (0: none given).
    pub ifindex: u32,
    /// The interface by local address (IPv4; unspecified: none given).
    pub iface: Ipv4Addr,
}

impl Membership {
    /// Whether `group` is a multicast address (224.0.0.0/4, ff00::/8).
    pub fn is_multicast(&self) -> bool {
        self.group.is_multicast()
    }
}

/// Add a membership (`IP_ADD_MEMBERSHIP`, `IPV6_ADD_MEMBERSHIP`): `EINVAL`
/// if the group is not a multicast address, `EADDRINUSE` if the socket is a
/// member of it on that interface already, `ENOBUFS` past
/// [`MAX_MEMBERSHIPS`].
pub fn join(list: &mut Vec<Membership>, m: Membership) -> Result<(), Errno> {
    if !m.is_multicast() {
        return Err(Errno::EINVAL);
    }
    if list.contains(&m) {
        return Err(Errno::EADDRINUSE);
    }
    if list.len() >= MAX_MEMBERSHIPS {
        return Err(Errno::ENOBUFS);
    }
    list.push(m);
    Ok(())
}

/// Drop a membership (`IP_DROP_MEMBERSHIP`, `IPV6_DROP_MEMBERSHIP`):
/// `EINVAL` for a group that is not multicast, `EADDRNOTAVAIL` if the
/// socket is not a member.
pub fn leave(list: &mut Vec<Membership>, m: &Membership) -> Result<(), Errno> {
    if !m.is_multicast() {
        return Err(Errno::EINVAL);
    }
    let at = list.iter().position(|x| x == m).ok_or(Errno::EADDRNOTAVAIL)?;
    list.remove(at);
    Ok(())
}

/// What an Internet socket keeps of these options.
#[derive(Clone, Copy, Debug)]
pub struct IpOptions {
    vals: [Option<i32>; COUNT],
    /// `IP_MULTICAST_IF`: the interface's address and index.
    pub multicast_if: (Ipv4Addr, u32),
    /// `IPV6_PKTINFO` as set: source address and interface index for sends.
    pub pktinfo6: (Ipv6Addr, u32),
    pub congestion: Congestion,
}

impl Default for IpOptions {
    fn default() -> IpOptions {
        IpOptions {
            vals: [None; COUNT],
            multicast_if: (Ipv4Addr::UNSPECIFIED, 0),
            pktinfo6: (Ipv6Addr::UNSPECIFIED, 0),
            congestion: Congestion::default(),
        }
    }
}

impl IpOptions {
    pub fn get(&self, o: StoredOpt) -> i32 {
        self.vals[o as usize].unwrap_or_else(|| o.default_value())
    }

    pub fn set(&mut self, o: StoredOpt, v: i32) {
        self.vals[o as usize] = Some(v);
    }

    fn on(&self, o: StoredOpt) -> bool {
        self.get(o) != 0
    }

    /// The ancillary data a datagram that arrived with this header gets
    /// under the receive flags of the socket: `dst` is its destination
    /// address (the family decides IPv4 or IPv6 messages), `tos` the type
    /// of service or traffic class, `ttl` the time to live or hop limit.
    pub fn recv_anc(&self, dst: Option<IpAddr>, ifindex: u32, tos: u8, ttl: u8) -> RecvAnc {
        use StoredOpt::*;
        // A datagram that did not come with an IP header (a ping reply)
        // has nothing to report.
        let Some(dst) = dst else { return RecvAnc::default() };
        let v6 = dst.is_ipv6();
        let (pkt, tc, hl) =
            if v6 { (Ipv6RecvPktInfo, Ipv6RecvTclass, Ipv6RecvHopLimit) } else { (IpPktInfo, IpRecvTos, IpRecvTtl) };
        RecvAnc {
            v6,
            pktinfo: self.on(pkt).then_some(PktInfo { ifindex, dst }),
            tos: self.on(tc).then_some(tos),
            ttl: self.on(hl).then_some(ttl),
        }
    }
}

/// `struct in_pktinfo` / `struct in6_pktinfo` of a received datagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PktInfo {
    pub ifindex: u32,
    /// The address the datagram was sent to.
    pub dst: IpAddr,
}

/// The ancillary data of a received datagram that the socket's receive
/// flags ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecvAnc {
    /// The datagram is IPv6: the messages are at level `IPPROTO_IPV6`.
    pub v6: bool,
    pub pktinfo: Option<PktInfo>,
    /// `IP_TOS` / `IPV6_TCLASS`.
    pub tos: Option<u8>,
    /// `IP_TTL` / `IPV6_HOPLIMIT`.
    pub ttl: Option<u8>,
}

impl RecvAnc {
    pub fn is_empty(&self) -> bool {
        self.pktinfo.is_none() && self.tos.is_none() && self.ttl.is_none()
    }

    /// The control messages, in the order ip(7) lists the receive
    /// options: packet info, then TTL / hop limit, then TOS / traffic
    /// class. `IP_PKTINFO`'s `ipi_spec_dst` is the destination address too
    /// (the local address the datagram was routed to).
    pub fn cmsgs(&self) -> Vec<Cmsg> {
        let mut out = Vec::new();
        let level = if self.v6 { abi::IPPROTO_IPV6 } else { abi::IPPROTO_IP };
        if let Some(p) = self.pktinfo {
            let mut data = Vec::new();
            match p.dst {
                IpAddr::V4(d) => {
                    data.extend_from_slice(&(p.ifindex as i32).to_le_bytes());
                    data.extend_from_slice(&d.octets());
                    data.extend_from_slice(&d.octets());
                    out.push(Cmsg { level, ty: abi::IP_PKTINFO, data });
                }
                IpAddr::V6(d) => {
                    data.extend_from_slice(&d.octets());
                    data.extend_from_slice(&p.ifindex.to_le_bytes());
                    out.push(Cmsg { level, ty: abi::IPV6_PKTINFO, data });
                }
            }
        }
        if let Some(t) = self.ttl {
            let ty = if self.v6 { abi::IPV6_HOPLIMIT } else { abi::IP_TTL };
            out.push(Cmsg { level, ty, data: i32::from(t).to_le_bytes().to_vec() });
        }
        if let Some(t) = self.tos {
            // ip(7): the IP_TOS message is one byte; ipv6(7): the
            // IPV6_TCLASS one an int.
            if self.v6 {
                out.push(Cmsg { level, ty: abi::IPV6_TCLASS, data: i32::from(t).to_le_bytes().to_vec() });
            } else {
                out.push(Cmsg { level, ty: abi::IP_TOS, data: alloc::vec![t] });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec;

    const ALL: [StoredOpt; COUNT] = {
        use StoredOpt::*;
        [
            IpPktInfo,
            IpRecvTos,
            IpRecvTtl,
            IpRecvOpts,
            IpRetOpts,
            IpMtuDiscover,
            IpFreebind,
            IpTransparent,
            IpBindAddressNoPort,
            IpMulticastTtl,
            IpMulticastLoop,
            Ipv6MulticastIf,
            Ipv6MulticastHops,
            Ipv6MulticastLoop,
            Ipv6RecvTclass,
            Ipv6RecvPktInfo,
            Ipv6RecvHopLimit,
            Ipv6MtuDiscover,
            Ipv6DontFrag,
            TcpFastopen,
            TcpFastopenConnect,
            TcpQuickack,
            TcpCork,
            TcpDeferAccept,
            TcpUserTimeout,
            TcpNotsentLowat,
        ]
    };

    fn v4(a: [u8; 4]) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(a))
    }

    #[test]
    fn every_option_has_its_own_slot_and_scope() {
        for (i, o) in ALL.iter().enumerate() {
            assert_eq!(*o as usize, i, "{o:?}");
        }
        let scopes: std::vec::Vec<Scope> = ALL.iter().map(|o| o.scope()).collect();
        assert_eq!(scopes.iter().filter(|s| **s == Scope::V4).count(), 11);
        assert_eq!(scopes.iter().filter(|s| **s == Scope::V6).count(), 8);
        assert_eq!(scopes.iter().filter(|s| **s == Scope::Tcp).count(), 7);
    }

    #[test]
    fn numbers_name_the_options() {
        let name = StoredOpt::from_linux;
        assert_eq!(name(abi::SOL_IP, 8), Some(StoredOpt::IpPktInfo));
        assert_eq!(name(abi::SOL_IP, 10), Some(StoredOpt::IpMtuDiscover));
        assert_eq!(name(abi::SOL_IP, 13), Some(StoredOpt::IpRecvTos));
        assert_eq!(name(abi::SOL_IP, 33), Some(StoredOpt::IpMulticastTtl));
        assert_eq!(name(abi::SOL_IPV6, 66), Some(StoredOpt::Ipv6RecvTclass));
        assert_eq!(name(abi::SOL_TCP, 23), Some(StoredOpt::TcpFastopen));
        // Structure-valued and acted-on options are not among them.
        assert_eq!(name(abi::SOL_IP, abi::IP_ADD_MEMBERSHIP), None);
        assert_eq!(name(abi::SOL_IP, abi::IP_TTL), None);
        assert_eq!(name(abi::SOL_IPV6, abi::IPV6_TCLASS), None);
        assert_eq!(name(abi::SOL_TCP, abi::TCP_CONGESTION), None);
        // Every option is reachable by one (level, number) pair at least.
        let mut seen = std::vec::Vec::new();
        for level in [abi::SOL_IP, abi::SOL_IPV6, abi::SOL_TCP] {
            for n in 0..80 {
                if let Some(o) = name(level, n) {
                    assert_eq!(
                        o.scope(),
                        match level {
                            abi::SOL_IP => Scope::V4,
                            abi::SOL_IPV6 => Scope::V6,
                            _ => Scope::Tcp,
                        }
                    );
                    seen.push(o as usize);
                }
            }
        }
        seen.sort_unstable();
        assert_eq!(seen, (0..COUNT).collect::<std::vec::Vec<_>>());
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let o = IpOptions::default();
        use StoredOpt::*;
        assert_eq!(o.get(IpMulticastTtl), 1, "ip(7): the default TTL of multicast is 1");
        assert_eq!(o.get(IpMulticastLoop), 1);
        assert_eq!(o.get(Ipv6MulticastHops), 1);
        assert_eq!(o.get(Ipv6MulticastLoop), 1);
        assert_eq!(o.get(TcpQuickack), 1);
        assert_eq!(o.get(IpMtuDiscover), 1, "IP_PMTUDISC_WANT");
        for z in [IpPktInfo, IpRecvTos, IpRecvTtl, IpFreebind, Ipv6RecvPktInfo, TcpFastopen, TcpCork, TcpUserTimeout] {
            assert_eq!(o.get(z), 0, "{z:?}");
        }
        assert_eq!(o.multicast_if, (Ipv4Addr::UNSPECIFIED, 0));
        assert_eq!(o.congestion.name(), "cubic");
    }

    #[test]
    fn values_are_checked_and_reduced() {
        use StoredOpt::*;
        // Flags: any non-zero value is on.
        for f in [IpPktInfo, IpRecvTos, IpRecvTtl, IpRecvOpts, IpRetOpts, IpFreebind, IpTransparent, TcpCork] {
            assert_eq!((f.normalize(0), f.normalize(5), f.normalize(-1)), (Ok(0), Ok(1), Ok(1)), "{f:?}");
        }
        assert_eq!(IpMtuDiscover.normalize(5), Ok(5));
        assert_eq!(IpMtuDiscover.normalize(6), Err(Errno::EINVAL));
        assert_eq!(IpMtuDiscover.normalize(-1), Err(Errno::EINVAL));
        assert_eq!(Ipv6MtuDiscover.normalize(3), Ok(3));
        assert_eq!(IpMulticastTtl.normalize(255), Ok(255));
        assert_eq!(IpMulticastTtl.normalize(256), Err(Errno::EINVAL));
        assert_eq!(IpMulticastTtl.normalize(-1), Err(Errno::EINVAL));
        assert_eq!(IpMulticastLoop.normalize(7), Ok(1));
        assert_eq!(Ipv6MulticastHops.normalize(-1), Ok(1), "-1: the default");
        assert_eq!(Ipv6MulticastHops.normalize(200), Ok(200));
        assert_eq!(Ipv6MulticastHops.normalize(256), Err(Errno::EINVAL));
        assert_eq!(Ipv6MulticastHops.normalize(-2), Err(Errno::EINVAL));
        assert_eq!(Ipv6MulticastLoop.normalize(1), Ok(1));
        assert_eq!(Ipv6MulticastLoop.normalize(2), Err(Errno::EINVAL));
        assert_eq!(TcpFastopenConnect.normalize(2), Err(Errno::EINVAL));
        assert_eq!(TcpFastopen.normalize(-1), Err(Errno::EINVAL));
        assert_eq!(TcpFastopen.normalize(256), Ok(256));
        assert_eq!(TcpUserTimeout.normalize(-1), Err(Errno::EINVAL));
        assert_eq!(TcpUserTimeout.normalize(30_000), Ok(30_000));
        assert_eq!(TcpDeferAccept.normalize(-4), Ok(0));
        assert_eq!(Ipv6MulticastIf.normalize(-1), Err(Errno::EINVAL));
        assert_eq!(Ipv6MulticastIf.normalize(2), Ok(2));
        assert_eq!(TcpNotsentLowat.normalize(16384), Ok(16384));
    }

    #[test]
    fn what_was_set_is_reported_back() {
        let mut o = IpOptions::default();
        for (i, opt) in ALL.iter().enumerate() {
            let v = opt.normalize(i as i32 % 2).unwrap_or(0);
            o.set(*opt, v);
            assert_eq!(o.get(*opt), v, "{opt:?}");
        }
        o.set(StoredOpt::IpMulticastLoop, 0);
        o.set(StoredOpt::IpMulticastTtl, 0);
        assert_eq!((o.get(StoredOpt::IpMulticastLoop), o.get(StoredOpt::IpMulticastTtl)), (0, 0));
    }

    #[test]
    fn congestion_names() {
        assert_eq!(Congestion::parse(b"cubic"), Ok(Congestion::Cubic));
        assert_eq!(Congestion::parse(b"reno\0xyz"), Ok(Congestion::Reno), "up to the first NUL");
        assert_eq!(Congestion::parse(b"cubic\0"), Ok(Congestion::Cubic));
        assert_eq!(Congestion::parse(b"bbr"), Err(Errno::ENOENT));
        assert_eq!(Congestion::parse(b"Cubic"), Err(Errno::ENOENT));
        assert_eq!(Congestion::parse(b""), Err(Errno::EINVAL));
        // Only the first 15 bytes count.
        assert_eq!(Congestion::parse(b"cubicxxxxxxxxxxxxxx"), Err(Errno::ENOENT));
    }

    #[test]
    fn memberships_are_a_set() {
        let g = |a, i, f| Membership { group: v4(a), ifindex: i, iface: Ipv4Addr::from(f) };
        let mut l = Vec::new();
        let a = g([224, 0, 0, 251], 0, [0; 4]);
        assert_eq!(join(&mut l, a), Ok(()));
        assert_eq!(join(&mut l, a), Err(Errno::EADDRINUSE), "joined twice");
        // The same group on another interface is another membership.
        assert_eq!(join(&mut l, g([224, 0, 0, 251], 2, [0; 4])), Ok(()));
        assert_eq!(join(&mut l, g([224, 0, 0, 251], 0, [10, 0, 2, 15])), Ok(()));
        assert_eq!(l.len(), 3);
        assert_eq!(join(&mut l, g([10, 0, 0, 1], 0, [0; 4])), Err(Errno::EINVAL), "not a multicast group");
        assert_eq!(leave(&mut l, &g([10, 0, 0, 1], 0, [0; 4])), Err(Errno::EINVAL));
        assert_eq!(leave(&mut l, &g([239, 1, 1, 1], 0, [0; 4])), Err(Errno::EADDRNOTAVAIL), "never joined");
        assert_eq!(leave(&mut l, &a), Ok(()));
        assert_eq!(leave(&mut l, &a), Err(Errno::EADDRNOTAVAIL), "left twice");
        assert_eq!(l.len(), 2);
        assert_eq!(join(&mut l, a), Ok(()), "can join again after leaving");
        // IPv6 groups.
        let six = Membership {
            group: "ff02::fb".parse::<Ipv6Addr>().unwrap().into(),
            ifindex: 2,
            iface: Ipv4Addr::UNSPECIFIED,
        };
        assert_eq!(join(&mut l, six), Ok(()));
        assert_eq!(join(&mut l, six), Err(Errno::EADDRINUSE));
        let uni = Membership { group: "fe80::1".parse::<Ipv6Addr>().unwrap().into(), ..six };
        assert_eq!(join(&mut l, uni), Err(Errno::EINVAL));
        // The limit.
        let mut l = Vec::new();
        for i in 0..MAX_MEMBERSHIPS as u8 {
            assert_eq!(join(&mut l, g([239, 0, 0, i], 0, [0; 4])), Ok(()));
        }
        assert_eq!(join(&mut l, g([239, 0, 1, 0], 0, [0; 4])), Err(Errno::ENOBUFS));
    }

    #[test]
    fn receive_options_select_the_control_messages() {
        use StoredOpt::*;
        let dst = v4([127, 0, 0, 1]);
        let mut o = IpOptions::default();
        assert!(o.recv_anc(Some(dst), 1, 0x28, 64).is_empty(), "nothing is asked for");
        o.set(IpPktInfo, 1);
        let a = o.recv_anc(Some(dst), 1, 0x28, 64);
        assert_eq!(a.pktinfo, Some(PktInfo { ifindex: 1, dst }));
        assert_eq!((a.tos, a.ttl), (None, None));
        let m = a.cmsgs();
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].level, m[0].ty), (abi::IPPROTO_IP, abi::IP_PKTINFO));
        // struct in_pktinfo: ifindex, spec_dst, addr.
        assert_eq!(m[0].data, vec![1, 0, 0, 0, 127, 0, 0, 1, 127, 0, 0, 1]);
        o.set(IpRecvTos, 1);
        o.set(IpRecvTtl, 1);
        let m = o.recv_anc(Some(dst), 2, 0xb8, 64).cmsgs();
        assert_eq!(m.len(), 3);
        assert_eq!((m[1].level, m[1].ty, m[1].data.as_slice()), (0, abi::IP_TTL, [64, 0, 0, 0].as_slice()));
        assert_eq!((m[2].level, m[2].ty, m[2].data.as_slice()), (0, abi::IP_TOS, [0xb8].as_slice()), "one byte");
        assert_eq!(m[0].data[..4], [2, 0, 0, 0], "the interface it came in on");
        // IPv6 flags do nothing on an IPv4 datagram, and the other way round.
        let mut o = IpOptions::default();
        o.set(Ipv6RecvPktInfo, 1);
        o.set(Ipv6RecvTclass, 1);
        o.set(Ipv6RecvHopLimit, 1);
        assert!(o.recv_anc(Some(dst), 1, 0, 64).is_empty());
        let six: Ipv6Addr = "::1".parse().unwrap();
        let a = o.recv_anc(Some(six.into()), 1, 0x2e, 255);
        let m = a.cmsgs();
        assert_eq!(m.len(), 3);
        assert_eq!((m[0].level, m[0].ty), (abi::IPPROTO_IPV6, abi::IPV6_PKTINFO));
        let mut want = six.octets().to_vec();
        want.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(m[0].data, want, "struct in6_pktinfo: address, ifindex");
        assert_eq!((m[1].ty, m[1].data.as_slice()), (abi::IPV6_HOPLIMIT, 255i32.to_le_bytes().as_slice()));
        assert_eq!((m[2].ty, m[2].data.as_slice()), (abi::IPV6_TCLASS, 0x2ei32.to_le_bytes().as_slice()), "an int");
        let mut o4 = IpOptions::default();
        o4.set(IpPktInfo, 1);
        assert!(o4.recv_anc(Some(six.into()), 1, 0, 64).is_empty());
        // A datagram without an IP header says nothing.
        assert!(o.recv_anc(None, 0, 0, 0).is_empty());
    }
}
