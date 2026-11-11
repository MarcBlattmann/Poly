//! The network devices of a network namespace and the interface requests
//! of ioctl(2) on a socket (netdevice(7)), ADR 0178.
//!
//! - [`Devices`] is what a network namespace keeps about its loopback
//!   device: whether `lo` is up and whether it has its IPv4 address. A new
//!   namespace's `lo` is down and has none (network_namespaces(7): "a new
//!   network namespace ... has only a loopback interface"; its spec sheet:
//!   flags `IFF_LOOPBACK` only, `SIOCGIFADDR` is `EADDRNOTAVAIL`). Bringing
//!   it up gives it 127.0.0.1/8, which it keeps when it goes down again,
//!   and ::1/128 for as long as it is up. The initial namespace's `lo`
//!   starts up with both; the kernel adds its other devices (`eth0`) to
//!   the [`Interfaces`] it reports.
//! - [`ifreq`] answers the `SIOCGIF*` requests from an [`Interfaces`]
//!   snapshot, and says what a `SIOCSIFFLAGS` asks for; [`ifconf`] is
//!   `SIOCGIFCONF`. The handlers (`sys::netdev`) copy the structures and
//!   check the privilege.
//! - [`proc_net_dev`] and [`proc_net_route`] are the texts of
//!   `/proc/<pid>/net/dev` and `/proc/<pid>/net/route` (proc_pid_net(5)).
//!
//! Request numbers from `linux/sockios.h`, `struct ifreq` and `struct
//! ifconf` from `linux/if.h`, `ARPHRD_*` from `linux/if_arp.h`.

use super::netlink::{ARPHRD_ETHER, ARPHRD_LOOPBACK, IfAddr, Interfaces, Link, LinkStats};
use crate::errno::Errno;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const SIOCGIFNAME: u32 = 0x8910;
pub const SIOCGIFCONF: u32 = 0x8912;
pub const SIOCGIFFLAGS: u32 = 0x8913;
pub const SIOCSIFFLAGS: u32 = 0x8914;
pub const SIOCGIFADDR: u32 = 0x8915;
pub const SIOCSIFADDR: u32 = 0x8916;
pub const SIOCGIFDSTADDR: u32 = 0x8917;
pub const SIOCSIFDSTADDR: u32 = 0x8918;
pub const SIOCGIFBRDADDR: u32 = 0x8919;
pub const SIOCSIFBRDADDR: u32 = 0x891a;
pub const SIOCGIFNETMASK: u32 = 0x891b;
pub const SIOCSIFNETMASK: u32 = 0x891c;
pub const SIOCGIFMETRIC: u32 = 0x891d;
pub const SIOCSIFMETRIC: u32 = 0x891e;
pub const SIOCGIFMTU: u32 = 0x8921;
pub const SIOCSIFMTU: u32 = 0x8922;
pub const SIOCSIFNAME: u32 = 0x8923;
pub const SIOCSIFHWADDR: u32 = 0x8924;
pub const SIOCGIFHWADDR: u32 = 0x8927;
pub const SIOCGIFINDEX: u32 = 0x8933;
pub const SIOCGIFTXQLEN: u32 = 0x8942;
pub const SIOCSIFTXQLEN: u32 = 0x8943;

/// `sizeof(struct ifreq)`: a 16-byte name and a 24-byte union.
pub const IFREQ_SIZE: usize = 40;
/// `IFNAMSIZ`.
pub const IFNAMSIZ: usize = 16;
/// `sizeof(struct ifconf)`: `int ifc_len`, padding, a pointer.
pub const IFCONF_SIZE: usize = 16;

/// The index of `lo` in every network namespace.
pub const LO_INDEX: u32 = 1;
/// The MTU of `lo`.
pub const LO_MTU: u32 = 65536;
/// What `SIOCGIFTXQLEN` and `IFLA_TXQLEN` report.
pub const TXQLEN: u32 = 1000;

/// The loopback device of a network namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Devices {
    lo_up: bool,
    /// `lo` has 127.0.0.1/8.
    lo_addr: bool,
}

impl Default for Devices {
    fn default() -> Devices {
        Devices::new()
    }
}

impl Devices {
    /// A new network namespace's: `lo` down, without an address.
    pub const fn new() -> Devices {
        Devices { lo_up: false, lo_addr: false }
    }

    /// The initial namespace's: `lo` up with its addresses.
    pub const fn initial() -> Devices {
        Devices { lo_up: true, lo_addr: true }
    }

    /// Whether `lo` is up (packets are routed to 127.0.0.0/8 and ::1).
    pub fn lo_up(&self) -> bool {
        self.lo_up
    }

    /// Bring `lo` up or down. Coming up gives it 127.0.0.1/8 if it has
    /// none; going down leaves that address on it.
    pub fn set_lo_up(&mut self, up: bool) {
        self.lo_up = up;
        if up {
            self.lo_addr = true;
        }
    }

    /// Give `lo` the address 127.0.0.1/8 while it is down (`RTM_NEWADDR`,
    /// as sandboxes do before they bring it up): `EEXIST` if it has it.
    pub fn add_lo_addr(&mut self) -> Result<(), Errno> {
        if self.lo_addr {
            return Err(Errno::EEXIST);
        }
        self.lo_addr = true;
        Ok(())
    }

    /// `lo` and its addresses, with the counters `stats`.
    pub fn interfaces(&self, stats: LinkStats) -> Interfaces {
        let mut ifs = Interfaces::default();
        ifs.links.push(Link {
            index: LO_INDEX,
            name: b"lo".to_vec(),
            loopback: true,
            mac: [0; 6],
            mtu: LO_MTU,
            up: self.lo_up,
            carrier: true,
            stats,
        });
        if self.lo_addr {
            ifs.addrs.push(IfAddr { index: LO_INDEX, addr: IpAddr::V4(Ipv4Addr::LOCALHOST), prefix_len: 8 });
        }
        if self.lo_up {
            ifs.addrs.push(IfAddr { index: LO_INDEX, addr: IpAddr::V6(Ipv6Addr::LOCALHOST), prefix_len: 128 });
        }
        ifs
    }
}

/// What an interface request does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfAnswer {
    /// The structure was filled in: copy it back.
    Filled,
    /// `SIOCSIFFLAGS`: bring the link with this index up or down. Needs
    /// `CAP_NET_ADMIN` (the caller checks, before this).
    SetUp { index: u32, up: bool },
}

/// Whether `request` is a request that changes a device (netdevice(7):
/// "Setting ... is a privileged operation").
pub const fn is_set_request(request: u32) -> bool {
    matches!(
        request,
        SIOCSIFFLAGS
            | SIOCSIFADDR
            | SIOCSIFDSTADDR
            | SIOCSIFBRDADDR
            | SIOCSIFNETMASK
            | SIOCSIFMETRIC
            | SIOCSIFMTU
            | SIOCSIFNAME
            | SIOCSIFHWADDR
            | SIOCSIFTXQLEN
    )
}

/// Whether `request` is one [`ifreq`] answers.
pub const fn is_ifreq_request(request: u32) -> bool {
    is_set_request(request)
        || matches!(
            request,
            SIOCGIFNAME
                | SIOCGIFFLAGS
                | SIOCGIFADDR
                | SIOCGIFDSTADDR
                | SIOCGIFBRDADDR
                | SIOCGIFNETMASK
                | SIOCGIFMETRIC
                | SIOCGIFMTU
                | SIOCGIFHWADDR
                | SIOCGIFINDEX
                | SIOCGIFTXQLEN
        )
}

/// The name in a `struct ifreq`: the bytes up to the first NUL.
fn name_of(req: &[u8; IFREQ_SIZE]) -> &[u8] {
    let n = &req[..IFNAMSIZ];
    n.split(|&b| b == 0).next().unwrap_or(n)
}

/// `struct sockaddr_in` for `addr` in the union of `req`.
fn put_inet(req: &mut [u8; IFREQ_SIZE], addr: Ipv4Addr) {
    req[16..].fill(0);
    req[16..18].copy_from_slice(&super::abi::AF_INET.to_le_bytes());
    req[20..24].copy_from_slice(&addr.octets());
}

fn netmask(prefix_len: u8) -> Ipv4Addr {
    let m = if prefix_len == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix_len.min(32))) };
    Ipv4Addr::from(m)
}

/// The first IPv4 address of link `index`.
fn ipv4_of(ifs: &Interfaces, index: u32) -> Option<(Ipv4Addr, u8)> {
    ifs.addrs.iter().find_map(|a| match a.addr {
        IpAddr::V4(v) if a.index == index => Some((v, a.prefix_len)),
        _ => None,
    })
}

/// One interface request on `req` (a `struct ifreq`), answered from `ifs`
/// (netdevice(7)):
///
/// - `SIOCGIFNAME`: the name of the link with index `ifr_ifindex`
///   (`ENODEV` if there is none).
/// - Every other request names the link in `ifr_name`: `ENODEV` for a
///   name no link has.
/// - `SIOCGIFINDEX`, `SIOCGIFFLAGS` (the low 16 bits of the link's flags),
///   `SIOCGIFMTU`, `SIOCGIFMETRIC` (0), `SIOCGIFTXQLEN`, `SIOCGIFHWADDR`
///   (`sa_family` is the `ARPHRD_*` type, `sa_data` the address: zeros for
///   `lo`).
/// - `SIOCGIFADDR`, `SIOCGIFNETMASK`, `SIOCGIFBRDADDR`, `SIOCGIFDSTADDR`:
///   a `sockaddr_in` from the link's IPv4 address; `EADDRNOTAVAIL` while
///   it has none.
/// - `SIOCSIFFLAGS`: the caller is to bring the link up or down as the
///   `IFF_UP` bit of `ifr_flags` says ([`IfAnswer::SetUp`]); the other
///   bits are the device's own and are not changed.
/// - The other setters: `EOPNOTSUPP` (Poly's devices keep their
///   addresses, names and sizes).
///
/// Anything else is `ENOTTY`.
pub fn ifreq(request: u32, req: &mut [u8; IFREQ_SIZE], ifs: &Interfaces) -> Result<IfAnswer, Errno> {
    if request == SIOCGIFNAME {
        let index = i32::from_le_bytes([req[16], req[17], req[18], req[19]]);
        let link = ifs.links.iter().find(|l| index > 0 && l.index == index as u32).ok_or(Errno::ENODEV)?;
        req[..IFNAMSIZ].fill(0);
        let n = link.name.len().min(IFNAMSIZ - 1);
        req[..n].copy_from_slice(&link.name[..n]);
        return Ok(IfAnswer::Filled);
    }
    if !is_ifreq_request(request) {
        return Err(Errno::ENOTTY);
    }
    let name = name_of(req);
    let link = ifs.links.iter().find(|l| l.name == name).ok_or(Errno::ENODEV)?;
    let addr = ipv4_of(ifs, link.index);
    match request {
        SIOCGIFINDEX => {
            req[16..].fill(0);
            req[16..20].copy_from_slice(&(link.index as i32).to_le_bytes());
        }
        SIOCGIFFLAGS => {
            req[16..].fill(0);
            req[16..18].copy_from_slice(&(link.flags() as u16).to_le_bytes());
        }
        SIOCGIFMTU => {
            req[16..].fill(0);
            req[16..20].copy_from_slice(&(link.mtu as i32).to_le_bytes());
        }
        SIOCGIFMETRIC => req[16..].fill(0),
        SIOCGIFTXQLEN => {
            req[16..].fill(0);
            req[16..20].copy_from_slice(&(TXQLEN as i32).to_le_bytes());
        }
        SIOCGIFHWADDR => {
            req[16..].fill(0);
            let ty = if link.loopback { ARPHRD_LOOPBACK } else { ARPHRD_ETHER };
            req[16..18].copy_from_slice(&ty.to_le_bytes());
            req[18..24].copy_from_slice(&link.mac);
        }
        SIOCGIFADDR | SIOCGIFDSTADDR => put_inet(req, addr.ok_or(Errno::EADDRNOTAVAIL)?.0),
        SIOCGIFNETMASK => put_inet(req, netmask(addr.ok_or(Errno::EADDRNOTAVAIL)?.1)),
        SIOCGIFBRDADDR => {
            let (a, len) = addr.ok_or(Errno::EADDRNOTAVAIL)?;
            let b = if link.loopback || len >= 31 {
                Ipv4Addr::UNSPECIFIED
            } else {
                Ipv4Addr::from(u32::from(a) | !u32::from(netmask(len)))
            };
            put_inet(req, b);
        }
        SIOCSIFFLAGS => {
            let flags = u16::from_le_bytes([req[16], req[17]]);
            return Ok(IfAnswer::SetUp { index: link.index, up: u32::from(flags) & super::netlink::IFF_UP != 0 });
        }
        _ => return Err(Errno::EOPNOTSUPP),
    }
    Ok(IfAnswer::Filled)
}

/// `SIOCGIFCONF` (netdevice(7)): the `struct ifreq`s of "all current L3
/// interface addresses that are running", IPv4 only: one per IPv4 address
/// of a link that is up, its name and a `sockaddr_in`. `room` is the
/// caller's `ifc_len` (`None` when `ifc_buf` is null: every entry is
/// counted); only whole entries that fit are returned.
pub fn ifconf(ifs: &Interfaces, room: Option<usize>) -> Vec<u8> {
    let mut out = Vec::new();
    for a in &ifs.addrs {
        let IpAddr::V4(v) = a.addr else { continue };
        let Some(link) = ifs.links.iter().find(|l| l.index == a.index && l.up) else { continue };
        if room.is_some_and(|r| out.len() + IFREQ_SIZE > r) {
            break;
        }
        let mut e = [0u8; IFREQ_SIZE];
        let n = link.name.len().min(IFNAMSIZ - 1);
        e[..n].copy_from_slice(&link.name[..n]);
        put_inet(&mut e, v);
        out.extend_from_slice(&e);
    }
    out
}

/// `/proc/<pid>/net/dev` (proc_pid_net(5)): two header lines and a line
/// per device with its receive and transmit counters.
pub fn proc_net_dev(ifs: &Interfaces) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(
        b"Inter-|   Receive                                                |  Transmit\n \
          face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n",
    );
    for l in &ifs.links {
        let s = &l.stats;
        let name = alloc::string::String::from_utf8_lossy(&l.name);
        out.extend_from_slice(
            alloc::format!(
                "{name:>6}: {:>7} {:>7} {:>4} {:>4} {:>4} {:>5} {:>10} {:>9} {:>8} {:>7} {:>4} {:>4} {:>4} {:>5} {:>7} {:>10}\n",
                s.rx_bytes, s.rx_packets, s.rx_errors, s.rx_dropped, 0, 0, 0, 0,
                s.tx_bytes, s.tx_packets, s.tx_errors, s.tx_dropped, 0, 0, 0, 0,
            )
            .as_bytes(),
        );
    }
    out
}

/// One line of `/proc/<pid>/net/route`: padded with spaces to 127
/// characters.
fn route_line(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(alloc::format!("{text:<127}\n").as_bytes());
}

/// `/proc/<pid>/net/route` (proc_pid_net(5)): the header line and a line
/// per IPv4 route of the main table (the subnets of the addresses of
/// links that are up and the default routes; the routes of a loopback
/// address live in the local table and are not listed). Addresses are
/// eight hexadecimal digits of the address in memory order.
pub fn proc_net_route(ifs: &Interfaces) -> Vec<u8> {
    use super::netlink::{RT_TABLE_MAIN, RTN_UNICAST};
    let hex = |a: IpAddr| match a {
        IpAddr::V4(v) => alloc::format!("{:08X}", u32::from_le_bytes(v.octets())),
        IpAddr::V6(_) => alloc::string::String::new(),
    };
    let mut out = Vec::new();
    route_line(&mut out, "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT");
    for r in ifs.routes() {
        if !r.dst.is_ipv4() || r.table != RT_TABLE_MAIN || r.kind != RTN_UNICAST {
            continue;
        }
        let Some(link) = ifs.links.iter().find(|l| l.index == r.oif && l.up) else { continue };
        let name = alloc::string::String::from_utf8_lossy(&link.name);
        // RTF_UP 1, RTF_GATEWAY 2 (`linux/route.h`).
        let flags = if r.gateway.is_some() { 3 } else { 1 };
        let gateway = r.gateway.map_or_else(|| alloc::string::String::from("00000000"), hex);
        let mask = hex(IpAddr::V4(netmask(r.dst_len)));
        route_line(
            &mut out,
            &alloc::format!("{name}\t{}\t{gateway}\t{flags:04X}\t0\t0\t0\t{mask}\t0\t0\t0", hex(r.dst)),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &[u8]) -> [u8; IFREQ_SIZE] {
        let mut r = [0u8; IFREQ_SIZE];
        r[..name.len()].copy_from_slice(name);
        r
    }

    fn int(r: &[u8; IFREQ_SIZE]) -> i32 {
        i32::from_le_bytes([r[16], r[17], r[18], r[19]])
    }

    fn short(r: &[u8; IFREQ_SIZE]) -> u16 {
        u16::from_le_bytes([r[16], r[17]])
    }

    fn with_eth() -> Interfaces {
        let mut ifs = Devices::initial().interfaces(LinkStats::default());
        ifs.links.push(Link {
            index: 2,
            name: b"eth0".to_vec(),
            loopback: false,
            mac: [0x52, 0x54, 0, 1, 2, 3],
            mtu: 1500,
            up: true,
            carrier: true,
            stats: LinkStats { rx_bytes: 1234, rx_packets: 5, tx_bytes: 99, tx_packets: 2, ..LinkStats::default() },
        });
        ifs.addrs.push(IfAddr { index: 2, addr: IpAddr::V4(Ipv4Addr::new(10, 0, 2, 15)), prefix_len: 24 });
        ifs.gateways.push((IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2)), 2));
        ifs
    }

    /// The sheet's new namespace: one device, down, no address.
    #[test]
    fn a_new_namespace_has_a_down_loopback_without_address() {
        let d = Devices::new();
        assert!(!d.lo_up());
        let ifs = d.interfaces(LinkStats::default());
        assert_eq!(ifs.links.len(), 1);
        assert!(ifs.addrs.is_empty());

        let mut r = req(b"lo");
        assert_eq!(ifreq(SIOCGIFFLAGS, &mut r, &ifs), Ok(IfAnswer::Filled));
        assert_eq!(short(&r), 0x8, "IFF_LOOPBACK only");
        assert_eq!(ifreq(SIOCGIFADDR, &mut req(b"lo"), &ifs), Err(Errno::EADDRNOTAVAIL));
        assert_eq!(ifreq(SIOCGIFNETMASK, &mut req(b"lo"), &ifs), Err(Errno::EADDRNOTAVAIL));
        let mut r = req(b"lo");
        ifreq(SIOCGIFINDEX, &mut r, &ifs).unwrap();
        assert_eq!(int(&r), 1);
        let mut r = req(b"lo");
        ifreq(SIOCGIFMTU, &mut r, &ifs).unwrap();
        assert_eq!(int(&r), 65536);
        let mut r = req(b"lo");
        ifreq(SIOCGIFHWADDR, &mut r, &ifs).unwrap();
        assert_eq!(short(&r), 772);
        assert_eq!(&r[18..24], &[0; 6]);
        // By index.
        let mut r = [0u8; IFREQ_SIZE];
        r[16..20].copy_from_slice(&1i32.to_le_bytes());
        ifreq(SIOCGIFNAME, &mut r, &ifs).unwrap();
        assert_eq!(&r[..3], b"lo\0");
        r[16..20].copy_from_slice(&2i32.to_le_bytes());
        assert_eq!(ifreq(SIOCGIFNAME, &mut r, &ifs), Err(Errno::ENODEV));
        r[16..20].copy_from_slice(&0i32.to_le_bytes());
        assert_eq!(ifreq(SIOCGIFNAME, &mut r, &ifs), Err(Errno::ENODEV));
        // An unknown name, for getters and setters.
        for request in [SIOCGIFFLAGS, SIOCGIFADDR, SIOCGIFINDEX, SIOCGIFMTU, SIOCGIFHWADDR, SIOCSIFFLAGS] {
            assert_eq!(ifreq(request, &mut req(b"eth0"), &ifs), Err(Errno::ENODEV), "{request:#x}");
        }
        assert!(ifconf(&ifs, Some(400)).is_empty());
        assert!(ifconf(&ifs, None).is_empty());
        assert_eq!(ifreq(0x89f0, &mut req(b"lo"), &ifs), Err(Errno::ENOTTY));
    }

    /// Up with 0x49, as bubblewrap does; down again with 0.
    #[test]
    fn bringing_lo_up_and_down() {
        let mut d = Devices::new();
        let ifs = d.interfaces(LinkStats::default());
        let mut r = req(b"lo");
        r[16..18].copy_from_slice(&0x49u16.to_le_bytes());
        assert_eq!(ifreq(SIOCSIFFLAGS, &mut r, &ifs), Ok(IfAnswer::SetUp { index: 1, up: true }));
        d.set_lo_up(true);
        let ifs = d.interfaces(LinkStats::default());
        let mut r = req(b"lo");
        ifreq(SIOCGIFFLAGS, &mut r, &ifs).unwrap();
        assert_eq!(short(&r), 0x49);
        let mut r = req(b"lo");
        ifreq(SIOCGIFADDR, &mut r, &ifs).unwrap();
        assert_eq!((short(&r), &r[20..24]), (2, &[127, 0, 0, 1][..]));
        let mut r = req(b"lo");
        ifreq(SIOCGIFNETMASK, &mut r, &ifs).unwrap();
        assert_eq!(&r[20..24], &[255, 0, 0, 0]);
        let mut r = req(b"lo");
        ifreq(SIOCGIFBRDADDR, &mut r, &ifs).unwrap();
        assert_eq!(&r[20..24], &[0, 0, 0, 0]);
        let conf = ifconf(&ifs, Some(400));
        assert_eq!(conf.len(), 40, "one entry");
        assert_eq!((&conf[..3], &conf[20..24]), (&b"lo\0"[..], &[127, 0, 0, 1][..]));
        assert_eq!(ifconf(&ifs, Some(39)).len(), 0, "no room for a whole entry");
        assert_eq!(ifconf(&ifs, None).len(), 40);
        assert!(ifs.addrs.iter().any(|a| a.addr == IpAddr::V6(Ipv6Addr::LOCALHOST)));

        let mut r = req(b"lo");
        assert_eq!(ifreq(SIOCSIFFLAGS, &mut r, &ifs), Ok(IfAnswer::SetUp { index: 1, up: false }));
        d.set_lo_up(false);
        let ifs = d.interfaces(LinkStats::default());
        let mut r = req(b"lo");
        ifreq(SIOCGIFFLAGS, &mut r, &ifs).unwrap();
        assert_eq!(short(&r), 0x8);
        assert!(ifconf(&ifs, Some(400)).is_empty(), "not running");
        assert!(ifreq(SIOCGIFADDR, &mut req(b"lo"), &ifs).is_ok(), "the address stays");
        assert_eq!(d.add_lo_addr(), Err(Errno::EEXIST));
        let mut fresh = Devices::new();
        assert_eq!(fresh.add_lo_addr(), Ok(()));
        assert!(!fresh.lo_up());
        assert_eq!(fresh.interfaces(LinkStats::default()).addrs.len(), 1);
    }

    #[test]
    fn an_ethernet_device() {
        let ifs = with_eth();
        let mut r = req(b"eth0");
        ifreq(SIOCGIFFLAGS, &mut r, &ifs).unwrap();
        assert_eq!(short(&r), 0x1043, "UP | BROADCAST | RUNNING | MULTICAST");
        let mut r = req(b"eth0");
        ifreq(SIOCGIFHWADDR, &mut r, &ifs).unwrap();
        assert_eq!((short(&r), &r[18..24]), (1, &[0x52, 0x54, 0, 1, 2, 3][..]));
        let mut r = req(b"eth0");
        ifreq(SIOCGIFBRDADDR, &mut r, &ifs).unwrap();
        assert_eq!(&r[20..24], &[10, 0, 2, 255]);
        let mut r = req(b"eth0");
        ifreq(SIOCGIFTXQLEN, &mut r, &ifs).unwrap();
        assert_eq!(int(&r), 1000);
        assert_eq!(ifreq(SIOCSIFMTU, &mut req(b"eth0"), &ifs), Err(Errno::EOPNOTSUPP));
        assert_eq!(ifconf(&ifs, Some(4096)).len(), 80);
        assert_eq!(ifconf(&ifs, Some(79)).len(), 40);
        assert!(is_set_request(SIOCSIFFLAGS) && !is_set_request(SIOCGIFFLAGS));
        assert!(is_ifreq_request(SIOCGIFHWADDR) && !is_ifreq_request(SIOCGIFCONF));
    }

    #[test]
    fn proc_net_files() {
        let text = proc_net_dev(&Devices::new().interfaces(LinkStats::default()));
        let lines: Vec<&[u8]> = text.split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 3, "two header lines and lo");
        assert!(lines[0].starts_with(b"Inter-|   Receive"));
        assert!(lines[1].starts_with(b" face |bytes    packets errs drop"));
        assert_eq!(
            core::str::from_utf8(lines[2]).unwrap(),
            "    lo:       0       0    0    0    0     0          0         0        0       0    0    0    0     0       0          0"
        );
        let text = proc_net_dev(&with_eth());
        let text = core::str::from_utf8(&text).unwrap();
        assert_eq!(text.lines().count(), 4);
        assert!(text.lines().nth(3).unwrap().starts_with("  eth0:    1234       5 "));

        // A namespace with lo up: only the header (127.0.0.0/8 is in the
        // local table).
        let mut d = Devices::new();
        d.set_lo_up(true);
        let route = proc_net_route(&d.interfaces(LinkStats::default()));
        assert_eq!(route.len(), 128);
        assert!(
            route.starts_with(b"Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT ")
        );
        let route = proc_net_route(&with_eth());
        let route = core::str::from_utf8(&route).unwrap();
        let lines: Vec<&str> = route.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.len() == 127));
        assert_eq!(lines[1].trim_end(), "eth0\t0002000A\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0");
        assert_eq!(lines[2].trim_end(), "eth0\t00000000\t0202000A\t0003\t0\t0\t0\t00000000\t0\t0\t0");
    }
}
