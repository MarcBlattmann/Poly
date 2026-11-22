//! `getaddrinfo` / `GetAddrInfoW` and `getnameinfo` / `GetNameInfoW`
//! without the network (Learn's pages of each, "addrinfo"): the hints
//! checked, the service turned into a port, the host classified (numeric,
//! `localhost`, the empty host of a passive or loopback lookup, or a name
//! for the stub resolver), and the result list built.
//!
//! Choices to be confirmed by the spec agent against real Windows:
//!
//! - **Order**: IPv6 before IPv4 when both families are allowed (the
//!   default prefix policy of RFC 6724 prefers `::1` to `127.0.0.1`).
//! - **Socket types**: one entry per address, with the hint's socket type
//!   and protocol as they are (observed on Windows: a zero protocol stays
//!   zero, a zero type stays zero); a numeric host's entries carry
//!   `AI_NUMERICHOST` in `ai_flags`, all others 0.
//! - **`localhost`** (and the machine's own name) are the loopback
//!   addresses without asking a server; there is no `hosts` file yet.
//! - **`getnameinfo`** gives `localhost` for loopback addresses and the
//!   numeric form for any other address (no reverse lookups yet), unless
//!   `NI_NAMEREQD` asks for a name (`WSAHOST_NOT_FOUND`).

use crate::consts::*;
use crate::text;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

/// The `ADDRINFO` hints that matter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hints {
    pub flags: i32,
    pub family: i32,
    pub socktype: i32,
    pub protocol: i32,
    /// `ai_addrlen`, `ai_canonname`, `ai_addr` or `ai_next` of the hints is not zero/NULL.
    pub reserved: bool,
}

/// One result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub flags: i32,
    pub family: i32,
    pub socktype: i32,
    pub protocol: i32,
    pub addr: SocketAddr,
}

/// What the host part asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Host {
    /// Addresses known without a lookup.
    Addrs(Vec<IpAddr>),
    /// The empty host string `""` (without `AI_NUMERICHOST` or `AI_PASSIVE`):
    /// resolves to the machine's own addresses.
    SelfAddrs,
    /// A name for the hosts file and stub resolver (already checked to be a plausible name).
    Lookup(String),
}

/// Learn: hints whose `ai_addrlen`, `ai_canonname`, `ai_addr` or `ai_next` are not zero are
/// `WSANO_RECOVERY` (`EAI_FAIL`); a family other than `AF_UNSPEC`,
/// `AF_INET` and `AF_INET6` is `WSAEAFNOSUPPORT`; a socket type other
/// than 0, `SOCK_STREAM`, `SOCK_DGRAM` and `SOCK_RAW` is
/// `WSAESOCKTNOSUPPORT`; `AI_CANONNAME` without a host name is
/// `WSAEINVAL`. Flag bits Windows does not know are not refused (observed: `0x40000000`, and
/// `AI_DNS_ONLY` and the other newer flags).
pub fn check_hints(h: &Hints, node: Option<&str>) -> Result<(), u32> {
    if h.reserved {
        return Err(EAI_FAIL);
    }
    if !matches!(h.family, AF_UNSPEC | AF_INET | AF_INET6) {
        return Err(EAI_FAMILY);
    }
    if !matches!(h.socktype, 0 | SOCK_STREAM | SOCK_DGRAM | SOCK_RAW) {
        return Err(EAI_SOCKTYPE);
    }
    if h.flags & AI_CANONNAME != 0 && node.is_none() {
        return Err(EAI_BADFLAGS);
    }
    Ok(())
}

/// Well-known services by name (IANA's service name and port registry,
/// RFC 6335): the same port for TCP and UDP here.
pub const SERVICES: &[(&str, u16)] = &[
    ("echo", 7),
    ("discard", 9),
    ("daytime", 13),
    ("ftp-data", 20),
    ("ftp", 21),
    ("ssh", 22),
    ("telnet", 23),
    ("smtp", 25),
    ("time", 37),
    ("domain", 53),
    ("tftp", 69),
    ("gopher", 70),
    ("finger", 79),
    ("http", 80),
    ("www", 80),
    ("kerberos", 88),
    ("pop3", 110),
    ("sunrpc", 111),
    ("nntp", 119),
    ("ntp", 123),
    ("imap", 143),
    ("snmp", 161),
    ("ldap", 389),
    ("https", 443),
    ("submission", 587),
    ("ldaps", 636),
    ("imaps", 993),
    ("pop3s", 995),
];

/// The ports of [`SERVICES`] that are also registered for UDP, as Windows' own services file has them
/// (`getnameinfo` with `NI_DGRAM` names only those: port 80 stays "80").
const UDP_PORTS: &[u16] = &[7, 9, 13, 37, 53, 69, 88, 111, 123, 161, 389];

/// The port of a service name (case-insensitive), if Poly knows it.
pub fn service_by_name(name: &str) -> Option<u16> {
    SERVICES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, p)| p)
}

/// The name of a port's service, if Poly knows it (the first name).
pub fn service_by_port(port: u16) -> Option<&'static str> {
    SERVICES.iter().find(|&&(_, p)| p == port).map(|&(n, _)| n)
}

/// The same for the datagram (UDP) protocol: only the services registered for it have a name.
pub fn service_by_port_udp(port: u16) -> Option<&'static str> {
    if UDP_PORTS.contains(&port) { service_by_port(port) } else { None }
}

/// The service part: absent is port 0, a decimal number is that port, a
/// name is looked up unless `AI_NUMERICSERV` (`WSATYPE_NOT_FOUND`
/// otherwise).
pub fn service_port(service: Option<&str>, flags: i32) -> Result<u16, u32> {
    let Some(s) = service else { return Ok(0) };
    if s.is_empty() {
        return Ok(0);
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        // A number above 65535 wraps (observed on Windows: "65536" is port 0).
        return s.parse::<u32>().map(|n| n as u16).map_err(|_| EAI_SERVICE);
    }
    if flags & AI_NUMERICSERV != 0 {
        return Err(EAI_NONAME);
    }
    service_by_name(s).ok_or(EAI_SERVICE)
}

/// A numeric host: IPv4 dotted decimal or IPv6 text, the latter with an
/// optional numeric scope (`fe80::1%3`).
pub fn numeric_host(s: &str) -> Option<(IpAddr, u32)> {
    if let Some(b) = text::pton4(s) {
        return Some((IpAddr::V4(Ipv4Addr::from(b)), 0));
    }
    let (addr, scope) = match s.split_once('%') {
        Some((a, sc)) => (a, sc.parse::<u32>().ok()?),
        None => (s, 0),
    };
    text::pton6(addr).map(|b| (IpAddr::V6(Ipv6Addr::from(b)), scope))
}

/// Is `name` this machine (`localhost`, or the host name `own`)?
pub fn is_local_name(name: &str, own: &str) -> bool {
    let n = name.strip_suffix('.').unwrap_or(name);
    n.eq_ignore_ascii_case("localhost") || (!own.is_empty() && n.eq_ignore_ascii_case(own))
}

/// The families a lookup returns, in order.
fn families(h: &Hints) -> &'static [i32] {
    match h.family {
        AF_INET => &[AF_INET],
        AF_INET6 => &[AF_INET6],
        _ => &[AF_INET6, AF_INET],
    }
}

fn loopback(af: i32) -> IpAddr {
    if af == AF_INET { IpAddr::V4(Ipv4Addr::LOCALHOST) } else { IpAddr::V6(Ipv6Addr::LOCALHOST) }
}

fn unspecified(af: i32) -> IpAddr {
    if af == AF_INET { IpAddr::V4(Ipv4Addr::UNSPECIFIED) } else { IpAddr::V6(Ipv6Addr::UNSPECIFIED) }
}

/// Classify the host part (`own` is the machine's host name). A numeric
/// host of the wrong family is `WSAHOST_NOT_FOUND`; a name (or `""`) with
/// `AI_NUMERICHOST` too. An empty string `""` without `AI_NUMERICHOST`
/// resolves to the machine's own addresses (or wildcard with `AI_PASSIVE`).
pub fn classify(node: Option<&str>, h: &Hints, own: &str) -> Result<(Host, u32), u32> {
    let fams = families(h);
    let Some(node) = node else {
        let pick = if h.flags & AI_PASSIVE != 0 { unspecified } else { loopback };
        return Ok((Host::Addrs(fams.iter().map(|&f| pick(f)).collect()), 0));
    };
    if node.is_empty() {
        if h.flags & AI_NUMERICHOST != 0 {
            return Err(EAI_NONAME);
        }
        if h.flags & AI_PASSIVE != 0 {
            return Ok((Host::Addrs(fams.iter().map(|&f| unspecified(f)).collect()), 0));
        }
        return Ok((Host::SelfAddrs, 0));
    }
    if let Some((ip, scope)) = numeric_host(node) {
        let af = if ip.is_ipv4() { AF_INET } else { AF_INET6 };
        if !fams.contains(&af) {
            return Err(EAI_NONAME);
        }
        return Ok((Host::Addrs(alloc::vec![ip]), scope));
    }
    if h.flags & AI_NUMERICHOST != 0 {
        return Err(EAI_NONAME);
    }
    if is_local_name(node, own) {
        return Ok((Host::Addrs(fams.iter().map(|&f| loopback(f)).collect()), 0));
    }
    if crate::dns::encode_name(node, &mut Vec::new()).is_err() {
        return Err(EAI_NONAME);
    }
    Ok((Host::Lookup(node.to_string()), 0))
}

/// The machine's own addresses for an empty node string `""`: prefers non-loopback
/// interface addresses for each allowed family and falls back to loopback when a
/// family has none.
pub fn self_addrs(if_addrs: &[(bool, IpAddr)], h: &Hints) -> Vec<IpAddr> {
    let mut out = Vec::new();
    for &af in families(h) {
        let mut found = false;
        for &(is_lo, ip) in if_addrs {
            if is_lo {
                continue;
            }
            let matches = (af == AF_INET && ip.is_ipv4()) || (af == AF_INET6 && ip.is_ipv6());
            if matches && !out.contains(&ip) {
                out.push(ip);
                found = true;
            }
        }
        if !found {
            let lo = loopback(af);
            if !out.contains(&lo) {
                out.push(lo);
            }
        }
    }
    out
}

/// DNS record types to query for `h`.
pub fn query_types(h: &Hints) -> &'static [u16] {
    match h.family {
        AF_INET => &[crate::dns::TYPE_A],
        AF_INET6 if h.flags & AI_V4MAPPED != 0 => &[crate::dns::TYPE_AAAA, crate::dns::TYPE_A],
        AF_INET6 => &[crate::dns::TYPE_AAAA],
        _ => &[crate::dns::TYPE_AAAA, crate::dns::TYPE_A],
    }
}

/// The WSA error `getaddrinfo` / `GetAddrInfoW` reports when a host lookup
/// finds no addresses (observed on Windows 11: `WSAHOST_NOT_FOUND` (11001) for
/// NXDOMAIN, missing family / NODATA, invalid names and timeouts alike).
pub fn dns_error_wsa(_e: crate::dns::DnsError) -> u32 {
    WSAHOST_NOT_FOUND
}

/// The result list for `addrs` (filtered by the hints' family, ordered
/// IPv6 first unless the family is fixed) with `port`; `numeric` when the host was an address
/// literal (`ai_flags` `AI_NUMERICHOST`). The socket type and protocol are the hints'.
///
/// For `AF_INET6` on a non-numeric name with `AI_V4MAPPED`, IPv4 addresses are
/// returned as IPv4-mapped IPv6 addresses when no native IPv6 addresses exist,
/// or in addition to them when `AI_ALL` is also set.
pub fn entries(addrs: &[IpAddr], port: u16, scope: u32, h: &Hints, numeric: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    let flags = if numeric { AI_NUMERICHOST } else { 0 };
    let push_unique = |out: &mut Vec<Entry>, af: i32, a: SocketAddr| {
        if !out.iter().any(|e| e.addr == a) {
            out.push(Entry { flags, family: af, socktype: h.socktype, protocol: h.protocol, addr: a });
        }
    };
    for &af in families(h) {
        for ip in addrs {
            match ip {
                IpAddr::V4(v) if af == AF_INET => {
                    push_unique(&mut out, AF_INET, SocketAddr::new(IpAddr::V4(*v), port));
                }
                IpAddr::V6(v) if af == AF_INET6 => {
                    push_unique(&mut out, AF_INET6, SocketAddr::V6(SocketAddrV6::new(*v, port, 0, scope)));
                }
                _ => {}
            }
        }
    }
    if !numeric && h.family == AF_INET6 && h.flags & AI_V4MAPPED != 0 && (out.is_empty() || h.flags & AI_ALL != 0) {
        for ip in addrs {
            if let IpAddr::V4(v) = ip {
                let mapped = SocketAddr::V6(SocketAddrV6::new(v.to_ipv6_mapped(), port, 0, 0));
                push_unique(&mut out, AF_INET6, mapped);
            }
        }
    }
    out
}

/// `getnameinfo`'s host part.
pub fn name_of_host(addr: &SocketAddr, flags: i32) -> Result<String, u32> {
    let ip = addr.ip();
    if flags & NI_NUMERICHOST == 0 && ip.is_loopback() {
        return Ok("localhost".into());
    }
    if flags & NI_NAMEREQD != 0 {
        return Err(EAI_NONAME);
    }
    Ok(match addr {
        SocketAddr::V6(v) if v.scope_id() != 0 => alloc::format!("{}%{}", v.ip(), v.scope_id()),
        _ => ip.to_string(),
    })
}

/// `getnameinfo`'s service part.
pub fn name_of_service(port: u16, flags: i32) -> String {
    if flags & NI_NUMERICSERV == 0
        && let Some(n) = if flags & NI_DGRAM != 0 { service_by_port_udp(port) } else { service_by_port(port) }
    {
        return n.into();
    }
    port.to_string()
}

// ----- WSALookupServiceBeginW's checks (`winsock2.h`: `NSPROTO`-less name spaces, `LUP_*`)

/// `WSASERVICE_NOT_FOUND` (`winerror.h`).
pub const WSASERVICE_NOT_FOUND: u32 = 10108;
/// `ERROR_INVALID_PARAMETER`: what the DNS and NTDS name spaces answer.
const ERROR_INVALID_PARAMETER: u32 = 87;
/// `NS_ALL`, `NS_DNS`, `NS_NETBT`, `NS_WINS`, `NS_NLA`, `NS_NTDS` (`winsock2.h`, `nspapi.h`).
const NS_ALL: u32 = 0;
const NS_DNS: u32 = 12;
const NS_NLA: u32 = 15;
const NS_NTDS: u32 = 32;
/// `LUP_CONTAINERS` and `LUP_RETURN_NAME` (`winsock2.h`): a query for the names of containers.
const LUP_CONTAINERS: u32 = 0x2;
const LUP_RETURN_NAME: u32 = 0x10;
/// The smallest `dwSize` of a `WSAQUERYSETW` Poly accepts (the structure up to its protocols).
pub const QUERY_SET_MIN_SIZE: u32 = 80;

/// What `WSALookupServiceBeginW` makes of a query set's `dwSize` and `dwNameSpace` and the control
/// flags (observed on Windows): a size below the structure's is `WSAEINVAL`; `NS_ALL` and `NS_NLA`
/// start a lookup (that has nothing to return on Poly), except that a query for containers by
/// name is `WSASERVICE_NOT_FOUND` for `NS_ALL` and `WSANO_DATA` for `NS_NLA`; `NS_DNS` and `NS_NTDS`
/// are `ERROR_INVALID_PARAMETER`; every other name space is `WSASERVICE_NOT_FOUND`.
pub fn lookup_begin(size: u32, namespace: u32, flags: u32) -> Result<(), u32> {
    if size < QUERY_SET_MIN_SIZE {
        return Err(WSAEINVAL);
    }
    let containers_by_name = flags & (LUP_CONTAINERS | LUP_RETURN_NAME) == (LUP_CONTAINERS | LUP_RETURN_NAME);
    match namespace {
        NS_ALL if containers_by_name => Err(WSASERVICE_NOT_FOUND),
        NS_NLA if containers_by_name => Err(WSANO_DATA),
        NS_ALL | NS_NLA => Ok(()),
        NS_DNS | NS_NTDS => Err(ERROR_INVALID_PARAMETER),
        _ => Err(WSASERVICE_NOT_FOUND),
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    #[test]
    fn lookup_begin_rules() {
        assert_eq!(lookup_begin(120, 0, 0xFF0), Ok(()));
        assert_eq!(lookup_begin(120, 15, 0), Ok(()));
        assert_eq!(lookup_begin(120, 0, 0x12), Err(WSASERVICE_NOT_FOUND));
        assert_eq!(lookup_begin(120, 15, 0x12), Err(WSANO_DATA));
        assert_eq!(lookup_begin(120, 12, 0xFF0), Err(87));
        assert_eq!(lookup_begin(120, 32, 0x12), Err(87));
        for ns in [13, 14, 99] {
            assert_eq!(lookup_begin(120, ns, 0xFF0), Err(WSASERVICE_NOT_FOUND));
            assert_eq!(lookup_begin(120, ns, 0x12), Err(WSASERVICE_NOT_FOUND));
        }
        assert_eq!(lookup_begin(8, 15, 0xFF0), Err(WSAEINVAL));
        assert_eq!(lookup_begin(0, 15, 0xFF0), Err(WSAEINVAL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(flags: i32, family: i32, socktype: i32) -> Hints {
        Hints { flags, family, socktype, protocol: 0, reserved: false }
    }

    #[test]
    fn hints() {
        assert_eq!(check_hints(&h(0, AF_UNSPEC, SOCK_STREAM), Some("x")), Ok(()));
        assert_eq!(check_hints(&h(0x4000_0000, 0, 0), Some("x")), Ok(()));
        assert_eq!(check_hints(&Hints { reserved: true, ..h(0, 0, 0) }, Some("x")), Err(EAI_FAIL));
        assert_eq!(check_hints(&h(0, 7, 0), Some("x")), Err(EAI_FAMILY));
        assert_eq!(check_hints(&h(0, 0, 9), Some("x")), Err(EAI_SOCKTYPE));
        assert_eq!(check_hints(&h(AI_CANONNAME, 0, 0), None), Err(EAI_BADFLAGS));
    }

    #[test]
    fn services() {
        assert_eq!(service_port(None, 0), Ok(0));
        assert_eq!(service_port(Some("8080"), 0), Ok(8080));
        assert_eq!(service_port(Some("65536"), 0), Ok(0));
        assert_eq!(service_port(Some("70000"), 0), Ok(4464));
        assert_eq!(service_port(Some("99999999999"), 0), Err(EAI_SERVICE));
        assert_eq!(service_port(Some("HTTP"), 0), Ok(80));
        assert_eq!(service_port(Some("http"), AI_NUMERICSERV), Err(EAI_NONAME));
        assert_eq!(service_port(Some("nosuch"), 0), Err(EAI_SERVICE));
        assert_eq!(name_of_service(443, 0), "https");
        assert_eq!(name_of_service(443, NI_NUMERICSERV), "443");
        assert_eq!(name_of_service(12345, 0), "12345");
        assert_eq!(name_of_service(80, NI_DGRAM), "80");
        assert_eq!(name_of_service(53, NI_DGRAM), "domain");
    }

    #[test]
    fn hosts() {
        let any = h(0, AF_UNSPEC, SOCK_STREAM);
        let (host, _) = classify(Some("localhost"), &any, "poly").unwrap();
        let Host::Addrs(a) = host else { panic!() };
        let e = entries(&a, 80, 0, &any, false);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].addr.to_string(), "[::1]:80");
        assert_eq!(e[1].addr.to_string(), "127.0.0.1:80");
        assert_eq!((e[1].protocol, e[1].flags), (0, 0));
        assert_eq!(e[1].socktype, SOCK_STREAM);

        let v4 = h(0, AF_INET, SOCK_DGRAM);
        let (host, _) = classify(Some("POLY."), &v4, "poly").unwrap();
        let Host::Addrs(a) = host else { panic!() };
        let e = entries(&a, 53, 0, &v4, false);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].addr.to_string(), "127.0.0.1:53");
        assert_eq!(e[0].protocol, 0);
        let udp = Hints { protocol: IPPROTO_UDP, ..h(0, AF_INET, 0) };
        assert_eq!(entries(&a, 53, 0, &udp, true)[0].protocol, IPPROTO_UDP);
        assert_eq!(entries(&a, 53, 0, &udp, true)[0].flags, AI_NUMERICHOST);

        let passive = h(AI_PASSIVE, AF_INET, SOCK_STREAM);
        let (host, _) = classify(None, &passive, "poly").unwrap();
        assert_eq!(host, Host::Addrs(alloc::vec![IpAddr::V4(Ipv4Addr::UNSPECIFIED)]));

        assert_eq!(classify(Some("::1"), &v4, "poly"), Err(EAI_NONAME));
        assert_eq!(classify(Some("example.com"), &h(AI_NUMERICHOST, 0, 0), "poly"), Err(EAI_NONAME));
        assert_eq!(classify(Some("example.com"), &any, "poly").unwrap().0, Host::Lookup("example.com".into()));
        assert_eq!(classify(Some("bad name"), &any, "poly"), Err(EAI_NONAME));
        let (host, scope) = classify(Some("fe80::1%4"), &any, "poly").unwrap();
        let Host::Addrs(a) = host else { panic!() };
        let e = entries(&a, 1, scope, &h(0, 0, 0), true);
        assert_eq!(e[0].addr.to_string(), "[fe80::1%4]:1");
        assert_eq!(e[0].socktype, 0);
        assert_eq!(e[0].protocol, 0);
    }

    #[test]
    fn names() {
        let lo: SocketAddr = "127.0.0.1:80".parse().unwrap();
        assert_eq!(name_of_host(&lo, 0).unwrap(), "localhost");
        assert_eq!(name_of_host(&lo, NI_NUMERICHOST).unwrap(), "127.0.0.1");
        let other: SocketAddr = "10.0.2.2:80".parse().unwrap();
        assert_eq!(name_of_host(&other, 0).unwrap(), "10.0.2.2");
        assert_eq!(name_of_host(&other, NI_NAMEREQD), Err(EAI_NONAME));
    }

    #[test]
    fn empty_name_and_not_localhost() {
        let any = h(0, AF_UNSPEC, SOCK_STREAM);
        assert_eq!(classify(Some(""), &any, "poly"), Ok((Host::SelfAddrs, 0)));
        assert_eq!(classify(Some(""), &h(AI_NUMERICHOST, AF_INET, SOCK_STREAM), "poly"), Err(EAI_NONAME));
        assert_eq!(
            classify(Some(""), &h(AI_PASSIVE, AF_INET, SOCK_STREAM), "poly"),
            Ok((Host::Addrs(alloc::vec![IpAddr::V4(Ipv4Addr::UNSPECIFIED)]), 0))
        );
        // Subdomains of localhost and localhost.localdomain are NOT answered locally.
        assert_eq!(
            classify(Some("polytest.localhost"), &any, "poly").unwrap().0,
            Host::Lookup("polytest.localhost".into())
        );
        assert_eq!(
            classify(Some("localhost.localdomain"), &any, "poly").unwrap().0,
            Host::Lookup("localhost.localdomain".into())
        );
        // self_addrs picks NIC addresses and falls back to loopback per family.
        let ifs = [
            (true, IpAddr::V4(Ipv4Addr::LOCALHOST)),
            (true, IpAddr::V6(Ipv6Addr::LOCALHOST)),
            (false, IpAddr::V4(Ipv4Addr::new(10, 0, 2, 15))),
        ];
        let a = self_addrs(&ifs, &any);
        assert_eq!(a, [IpAddr::V6(Ipv6Addr::LOCALHOST), IpAddr::V4(Ipv4Addr::new(10, 0, 2, 15))]);
        assert_eq!(self_addrs(&[], &any), [IpAddr::V6(Ipv6Addr::LOCALHOST), IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    }

    #[test]
    fn family_filtering_and_v4mapped_all_rules() {
        let ip4 = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 55));
        let ip6 = IpAddr::V6("2001:db8::55".parse().unwrap());
        let mapped = IpAddr::V6("::ffff:192.0.2.55".parse().unwrap());

        // IPv4-only name.
        assert_eq!(entries(&[ip4], 0, 0, &h(0, AF_UNSPEC, SOCK_STREAM), false).len(), 1);
        assert_eq!(entries(&[ip4], 0, 0, &h(0, AF_INET, SOCK_STREAM), false).len(), 1);
        assert!(entries(&[ip4], 0, 0, &h(0, AF_INET6, SOCK_STREAM), false).is_empty());
        let e = entries(&[ip4], 0, 0, &h(AI_V4MAPPED, AF_INET6, SOCK_STREAM), false);
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].family, e[0].addr.ip()), (AF_INET6, mapped));
        let e = entries(&[ip4], 0, 0, &h(AI_V4MAPPED | AI_ALL, AF_INET6, SOCK_STREAM), false);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].addr.ip(), mapped);
        // Numeric IPv4 does NOT map to IPv6 even with AI_V4MAPPED.
        assert!(entries(&[ip4], 0, 0, &h(AI_V4MAPPED, AF_INET6, SOCK_STREAM), true).is_empty());
        assert_eq!(classify(Some("127.0.0.1"), &h(AI_V4MAPPED, AF_INET6, SOCK_STREAM), "poly"), Err(EAI_NONAME));

        // IPv6-only name.
        assert_eq!(entries(&[ip6], 0, 0, &h(0, AF_UNSPEC, SOCK_STREAM), false).len(), 1);
        assert_eq!(entries(&[ip6], 0, 0, &h(0, AF_INET6, SOCK_STREAM), false).len(), 1);
        assert!(entries(&[ip6], 0, 0, &h(0, AF_INET, SOCK_STREAM), false).is_empty());
        assert_eq!(entries(&[ip6], 0, 0, &h(AI_V4MAPPED, AF_INET6, SOCK_STREAM), false).len(), 1);
        assert_eq!(entries(&[ip6], 0, 0, &h(AI_ADDRCONFIG, AF_UNSPEC, SOCK_STREAM), false).len(), 1);

        // Both IPv4 and IPv6.
        let both = [ip4, ip6];
        assert_eq!(entries(&both, 0, 0, &h(0, AF_UNSPEC, SOCK_STREAM), false).len(), 2);
        assert_eq!(entries(&both, 0, 0, &h(0, AF_INET, SOCK_STREAM), false).len(), 1);
        assert_eq!(entries(&both, 0, 0, &h(0, AF_INET6, SOCK_STREAM), false).len(), 1);
        // AI_V4MAPPED alone: native IPv6 only (mapped IPv4 is not added).
        let e = entries(&both, 0, 0, &h(AI_V4MAPPED, AF_INET6, SOCK_STREAM), false);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].addr.ip(), ip6);
        // AI_V4MAPPED | AI_ALL: both native IPv6 and mapped IPv4.
        let e = entries(&both, 0, 0, &h(AI_V4MAPPED | AI_ALL, AF_INET6, SOCK_STREAM), false);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].addr.ip(), ip6);
        assert_eq!(e[1].addr.ip(), mapped);

        // DNS query types.
        assert_eq!(query_types(&h(0, AF_INET, SOCK_STREAM)), &[crate::dns::TYPE_A]);
        assert_eq!(query_types(&h(0, AF_INET6, SOCK_STREAM)), &[crate::dns::TYPE_AAAA]);
        assert_eq!(query_types(&h(AI_V4MAPPED, AF_INET6, SOCK_STREAM)), &[crate::dns::TYPE_AAAA, crate::dns::TYPE_A]);
        assert_eq!(query_types(&h(0, AF_UNSPEC, SOCK_STREAM)), &[crate::dns::TYPE_AAAA, crate::dns::TYPE_A]);
    }
}
